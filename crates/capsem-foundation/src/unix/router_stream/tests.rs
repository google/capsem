use super::*;
use tokio::io::{duplex, AsyncReadExt, AsyncWriteExt};
use tokio::time::{sleep, timeout};

#[tokio::test(start_paused = true)]
async fn half_closed_peer_has_a_deadline() {
    let (mut client, mut source) = duplex(32);
    let (mut server, mut destination) = duplex(32);
    let relay = tokio::spawn(async move { copy(&mut source, &mut destination, Limits::default()).await });
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
async fn stalled_writer_has_a_deadline_and_propagates_backpressure() {
    let (mut client, mut source) = duplex(32);
    let (_server, mut destination) = duplex(32);
    let relay = tokio::spawn(async move { copy(&mut source, &mut destination, Limits::default()).await });
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
    let relay = tokio::spawn(async move { copy(&mut source, &mut destination, Limits::default()).await });
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
    let relay = tokio::spawn(async move { copy(&mut source, &mut destination, Limits::default()).await });
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
    let relay = tokio::spawn(async move { copy(&mut source, &mut destination, Limits::default()).await });
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

#[tokio::test]
async fn cancelling_copy_releases_both_endpoints() {
    let (mut client, mut source) = duplex(32);
    let (mut server, mut destination) = duplex(32);
    let relay = tokio::spawn(async move { copy(&mut source, &mut destination, Limits::default()).await });
    client.write_all(b"ready").await.unwrap();
    server.read_exact(&mut [0; 5]).await.unwrap();
    relay.abort();
    assert!(relay.await.unwrap_err().is_cancelled());
    assert_eq!(client.read(&mut [0]).await.unwrap(), 0);
    assert_eq!(server.read(&mut [0]).await.unwrap(), 0);
}
