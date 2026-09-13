use super::*;
use tokio::io::{duplex, AsyncReadExt, AsyncWriteExt};
use tokio::time::{sleep, timeout};

#[tokio::test(start_paused = true)]
async fn cooperative_abort_reports_delivered_bytes_before_closing() {
    let (mut client, mut source) = duplex(32);
    let (mut server, mut destination) = duplex(32);
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let relay = tokio::spawn(async move {
        copy_until(&mut source, &mut destination, Framings::RAW, Limits::default(), async {
            let _ = stopped.await;
        })
        .await
    });
    client.write_all(b"request").await.unwrap();
    server.read_exact(&mut [0; 7]).await.unwrap();
    server.write_all(b"reply").await.unwrap();
    client.read_exact(&mut [0; 5]).await.unwrap();
    stop.send(()).unwrap();
    let outcome = timeout(Duration::from_secs(1), relay)
        .await
        .expect("cancellation must finish even when both peers stay open")
        .unwrap();
    assert_eq!(outcome.reason, CloseReason::Cancelled);
    assert_eq!((outcome.from_source, outcome.to_source), (7, 5));
    assert!(outcome.error.is_none());
    assert_eq!(client.read(&mut [0]).await.unwrap(), 0);
    assert_eq!(server.read(&mut [0]).await.unwrap(), 0);
}

#[tokio::test(start_paused = true)]
async fn half_closed_peer_has_a_deadline() {
    let (mut client, mut source) = duplex(32);
    let (mut server, mut destination) = duplex(32);
    let relay =
        tokio::spawn(async move { copy(&mut source, &mut destination, Framings::RAW, Limits::default()).await });
    client.shutdown().await.unwrap();
    assert_eq!(server.read(&mut [0]).await.unwrap(), 0);
    let result = timeout(Duration::from_secs(61), relay)
        .await
        .expect("half-closed relay remained alive")
        .unwrap();
    assert_eq!(result.reason, CloseReason::HalfCloseTimeout);
    assert_eq!((result.from_source, result.to_source), (0, 0));
}

#[tokio::test(start_paused = true)]
async fn cooperative_abort_interrupts_a_half_close_drain() {
    let (mut client, mut source) = duplex(32);
    let (mut server, mut destination) = duplex(32);
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let relay = tokio::spawn(async move {
        copy_until(&mut source, &mut destination, Framings::RAW, Limits::default(), async {
            let _ = stopped.await;
        })
        .await
    });
    client.write_all(b"request").await.unwrap();
    client.shutdown().await.unwrap();
    server.read_exact(&mut [0; 7]).await.unwrap();
    assert_eq!(server.read(&mut [0]).await.unwrap(), 0);
    stop.send(()).unwrap();
    let outcome = timeout(Duration::from_secs(1), relay).await.unwrap().unwrap();
    assert_eq!(outcome.reason, CloseReason::Cancelled);
    assert_eq!((outcome.from_source, outcome.to_source), (7, 0));
}

#[tokio::test(start_paused = true)]
async fn stalled_writer_has_a_deadline_and_propagates_backpressure() {
    let (mut client, mut source) = duplex(32);
    let (_server, mut destination) = duplex(32);
    let relay =
        tokio::spawn(async move { copy(&mut source, &mut destination, Framings::RAW, Limits::default()).await });
    let writer = tokio::spawn(async move { client.write_all(&vec![7; BUFFER_SIZE * 4]).await });
    sleep(Duration::from_secs(59)).await;
    assert!(
        !writer.is_finished(),
        "relay consumed unbounded input while the peer stalled"
    );
    let result = timeout(Duration::from_secs(2), relay)
        .await
        .expect("stalled relay remained alive")
        .unwrap();
    assert_eq!(result.reason, CloseReason::WriteStall);
    assert_eq!(result.from_source, 32);
    assert!(writer.await.unwrap().is_err());
}

#[tokio::test(start_paused = true)]
async fn quiet_connection_survives_and_preserves_both_directions() {
    let (mut client, mut source) = duplex(32);
    let (mut server, mut destination) = duplex(32);
    let relay =
        tokio::spawn(async move { copy(&mut source, &mut destination, Framings::RAW, Limits::default()).await });
    sleep(Duration::from_secs(120)).await;
    assert!(!relay.is_finished(), "quiet connected peer was treated as stalled");
    client.write_all(b"\0\xffrequest").await.unwrap();
    client.shutdown().await.unwrap();
    let mut request = Vec::new();
    server.read_to_end(&mut request).await.unwrap();
    assert_eq!(request, b"\0\xffrequest");
    server.write_all(b"response\0\xff").await.unwrap();
    server.shutdown().await.unwrap();
    let mut response = Vec::new();
    client.read_to_end(&mut response).await.unwrap();
    assert_eq!(response, b"response\0\xff");
    let result = relay.await.unwrap();
    assert_eq!(result.reason, CloseReason::Complete);
    assert_eq!((result.from_source, result.to_source), (9, 10));
    assert!(result.error.is_none());
}

#[tokio::test(start_paused = true)]
async fn slow_progress_renews_write_deadline_and_drains_trailing_bytes() {
    let (mut client, mut source) = duplex(BUFFER_SIZE * 2);
    let (mut server, mut destination) = duplex(32);
    let relay =
        tokio::spawn(async move { copy(&mut source, &mut destination, Framings::RAW, Limits::default()).await });
    let expected: Vec<u8> = (0..128).collect();
    client.write_all(&expected).await.unwrap();
    client.shutdown().await.unwrap();
    let mut received = Vec::new();
    for _ in 0..4 {
        sleep(Duration::from_secs(50)).await;
        assert!(!relay.is_finished(), "progress did not renew the write deadline");
        let mut chunk = [0; 32];
        server.read_exact(&mut chunk).await.unwrap();
        received.extend_from_slice(&chunk);
        tokio::task::yield_now().await;
    }
    assert_eq!(received, expected);
    assert_eq!(server.read(&mut [0]).await.unwrap(), 0);
    server.shutdown().await.unwrap();
    let result = relay.await.unwrap();
    assert_eq!(result.reason, CloseReason::Complete);
    assert_eq!(result.from_source, 128);
}

#[tokio::test(start_paused = true)]
async fn abrupt_peer_loss_preserves_delivered_count_and_stops_reverse_copy() {
    let (mut client, mut source) = duplex(32);
    let (mut server, mut destination) = duplex(32);
    let relay =
        tokio::spawn(async move { copy(&mut source, &mut destination, Framings::RAW, Limits::default()).await });
    client.write_all(b"delivered").await.unwrap();
    server.read_exact(&mut [0; 9]).await.unwrap();
    drop(server);
    client.write_all(b"undeliverable").await.unwrap();
    let result = relay.await.unwrap();
    assert_eq!(result.reason, CloseReason::Reset);
    assert_eq!(result.from_source, 9);
    assert_eq!(result.error.unwrap().kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(client.read(&mut [0]).await.unwrap(), 0);
}

const TO_VSOCK: Framings = Framings {
    source: Framing::Raw,
    destination: Framing::Framed,
};

fn frame(payload: &[u8]) -> Vec<u8> {
    let mut bytes = (payload.len() as u32).to_be_bytes().to_vec();
    bytes.extend_from_slice(payload);
    bytes
}

/// A framed leg carries end-of-stream as a zero-length frame and never as a
/// socket shutdown, so no transport can deliver the close ahead of the bytes;
/// the other direction keeps flowing after it.
#[tokio::test(start_paused = true)]
async fn a_framed_leg_carries_end_of_stream_in_band_and_keeps_the_other_direction() {
    let (mut client, mut source) = duplex(64);
    let (mut vsock, mut destination) = duplex(64);
    let relay = tokio::spawn(async move { copy(&mut source, &mut destination, TO_VSOCK, Limits::default()).await });
    client.write_all(b"request").await.unwrap();
    client.shutdown().await.unwrap();
    let mut wire = [0u8; 15];
    vsock.read_exact(&mut wire).await.unwrap();
    assert_eq!(wire.to_vec(), [frame(b"request"), frame(b"")].concat());
    assert!(
        timeout(Duration::from_secs(1), vsock.read(&mut [0])).await.is_err(),
        "the leg itself stays open after the end-of-stream frame"
    );
    vsock
        .write_all(&[frame(b"slow "), frame(b"reply"), frame(b"")].concat())
        .await
        .unwrap();
    let mut reply = Vec::new();
    client.read_to_end(&mut reply).await.unwrap();
    assert_eq!(reply, b"slow reply");
    let result = relay.await.unwrap();
    assert_eq!(result.reason, CloseReason::Complete);
    assert_eq!((result.from_source, result.to_source), (7, 10));
}

#[tokio::test(start_paused = true)]
async fn frames_split_anywhere_across_reads_decode_to_the_same_bytes() {
    let (mut vsock, mut source) = duplex(3);
    let (mut server, mut destination) = duplex(64);
    let framings = Framings {
        source: Framing::Framed,
        destination: Framing::Raw,
    };
    let relay = tokio::spawn(async move { copy(&mut source, &mut destination, framings, Limits::default()).await });
    let wire = [frame(b"ab"), frame(b"\0\0\0\0payload"), frame(b"")].concat();
    for byte in wire {
        vsock.write_all(&[byte]).await.unwrap();
    }
    let mut received = Vec::new();
    server.read_to_end(&mut received).await.unwrap();
    assert_eq!(received, b"ab\0\0\0\0payload");
    server.shutdown().await.unwrap();
    let mut back = [0u8; 4];
    vsock.read_exact(&mut back).await.unwrap();
    assert_eq!(back, [0; 4], "the raw close goes back as an end-of-stream frame");
    assert_eq!(relay.await.unwrap().reason, CloseReason::Complete);
}

/// Owner to owner, both legs framed: payloads larger than the buffer and
/// split across reads are re-framed without a byte moving or changing.
#[tokio::test(start_paused = true)]
async fn framed_to_framed_carries_large_payloads_exactly() {
    let (mut alpha, mut source) = duplex(BUFFER_SIZE);
    let (mut beta, mut destination) = duplex(BUFFER_SIZE);
    let framings = Framings {
        source: Framing::Framed,
        destination: Framing::Framed,
    };
    let relay = tokio::spawn(async move { copy(&mut source, &mut destination, framings, Limits::default()).await });
    let payload: Vec<u8> = (0..BUFFER_SIZE * 3 + 7).map(|i| (i * 31 % 251) as u8).collect();
    let wire = [frame(&payload[..40_000]), frame(&payload[40_000..]), frame(b"")].concat();
    let (sent, received) = tokio::join!(
        async {
            for piece in wire.chunks(5_003) {
                alpha.write_all(piece).await.unwrap();
            }
        },
        async {
            let (mut decoded, mut header) = (Vec::new(), [0u8; 4]);
            loop {
                beta.read_exact(&mut header).await.unwrap();
                let length = u32::from_be_bytes(header) as usize;
                if length == 0 {
                    return decoded;
                }
                let start = decoded.len();
                decoded.resize(start + length, 0);
                beta.read_exact(&mut decoded[start..]).await.unwrap();
            }
        }
    );
    let () = sent;
    assert_eq!(received, payload);
    beta.write_all(&frame(b"")).await.unwrap();
    let result = relay.await.unwrap();
    assert_eq!(result.reason, CloseReason::Complete);
    assert_eq!(result.from_source, payload.len() as u64);
}

#[tokio::test(start_paused = true)]
async fn a_framed_leg_that_ends_without_its_end_frame_was_cut_not_finished() {
    let (mut vsock, mut source) = duplex(64);
    let (mut server, mut destination) = duplex(64);
    let framings = Framings {
        source: Framing::Framed,
        destination: Framing::Raw,
    };
    let relay = tokio::spawn(async move { copy(&mut source, &mut destination, framings, Limits::default()).await });
    vsock.write_all(&frame(b"partial")).await.unwrap();
    server.read_exact(&mut [0; 7]).await.unwrap();
    drop(vsock);
    let result = relay.await.unwrap();
    assert_eq!(result.reason, CloseReason::Reset);
    assert_eq!(result.from_source, 7);
}

#[tokio::test(start_paused = true)]
async fn bytes_after_an_end_frame_are_refused() {
    let (mut vsock, mut source) = duplex(64);
    let (_server, mut destination) = duplex(64);
    let framings = Framings {
        source: Framing::Framed,
        destination: Framing::Raw,
    };
    let relay = tokio::spawn(async move { copy(&mut source, &mut destination, framings, Limits::default()).await });
    vsock.write_all(&[frame(b""), frame(b"late")].concat()).await.unwrap();
    let result = relay.await.unwrap();
    assert_eq!(result.reason, CloseReason::Io);
    assert_eq!(result.error.unwrap().kind(), io::ErrorKind::InvalidData);
}

#[tokio::test]
async fn cancelling_copy_releases_both_endpoints() {
    let (mut client, mut source) = duplex(32);
    let (mut server, mut destination) = duplex(32);
    let relay =
        tokio::spawn(async move { copy(&mut source, &mut destination, Framings::RAW, Limits::default()).await });
    client.write_all(b"ready").await.unwrap();
    server.read_exact(&mut [0; 5]).await.unwrap();
    relay.abort();
    assert!(relay.await.unwrap_err().is_cancelled());
    assert_eq!(client.read(&mut [0]).await.unwrap(), 0);
    assert_eq!(server.read(&mut [0]).await.unwrap(), 0);
}
