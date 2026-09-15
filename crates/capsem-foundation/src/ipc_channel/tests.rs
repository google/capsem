use super::*;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::time::Duration;

#[tokio::test]
async fn wire_is_one_bounded_big_endian_msgpack_frame() {
    let (channel, mut peer) = UnixStream::pair().unwrap();
    channel.set_nonblocking(true).unwrap();
    let (tx, _rx) = channel_from_std::<String, String>(channel).unwrap();
    let expected = rmp_serde::to_vec_named("hello").unwrap();

    let frame_len = 4 + expected.len();
    let read = tokio::task::spawn_blocking(move || {
        let mut frame = vec![0; frame_len];
        peer.read_exact(&mut frame).unwrap();
        frame
    });
    let sent = tokio::spawn(async move { tx.send("hello".to_string()).await.unwrap() });
    let frame = read.await.unwrap();
    sent.await.unwrap();

    assert_eq!(&frame[..4], &(expected.len() as u32).to_be_bytes());
    assert_eq!(&frame[4..], expected);
}

#[tokio::test]
async fn oversized_frame_is_rejected_before_payload_allocation() {
    let (channel, mut peer) = UnixStream::pair().unwrap();
    channel.set_nonblocking(true).unwrap();
    let (_tx, rx) = channel_from_std::<String, String>(channel).unwrap();
    peer.write_all(&u32::MAX.to_be_bytes()).unwrap();
    peer.shutdown(std::net::Shutdown::Write).unwrap();

    let error = rx.recv().await.unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("frame too large"), "{error}");
}

#[tokio::test]
async fn oversized_outbound_frame_is_rejected_before_writing() {
    let (channel, _peer) = UnixStream::pair().unwrap();
    let (tx, _rx) = channel_from_std::<String, String>(channel).unwrap();

    let error = tx.send("x".repeat(MAX_IPC_FRAME_SIZE as usize)).await.unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("frame too large"), "{error}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_senders_deliver_complete_non_interleaved_frames() {
    let (left, right) = UnixStream::pair().unwrap();
    let (tx, _left_rx) = channel_from_std::<String, String>(left).unwrap();
    let (_right_tx, rx) = channel_from_std::<String, String>(right).unwrap();
    let tx = Arc::new(tx);
    let mut sends = Vec::new();
    for id in 0..32 {
        let tx = Arc::clone(&tx);
        sends.push(tokio::spawn(async move {
            tx.send(format!("{id}:{}", "x".repeat(256 * 1024))).await.unwrap();
        }));
    }

    let mut ids = std::collections::HashSet::new();
    for _ in 0..32 {
        let message = rx.recv().await.unwrap();
        let (id, payload) = message.split_once(':').unwrap();
        assert_eq!(payload.len(), 256 * 1024);
        ids.insert(id.parse::<u8>().unwrap());
    }
    for send in sends {
        send.await.unwrap();
    }
    assert_eq!(ids.len(), 32);
}

/// Thirty-two connections at once, four hundred rounds, every reply
/// delivered. Over the transport's own teardown this failed by round
/// thirty on macOS: a closed connection's descriptor number was reused by
/// a new one before the old registration had left the kqueue, and the new
/// channel's `recv` never woke.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn churned_channels_keep_delivering() {
    let temp = tempfile::tempdir().unwrap();
    let socket = temp.path().join("ipc.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (tx, rx) = channel_from_std::<String, String>(stream.into_std().unwrap()).unwrap();
                while let Ok(message) = rx.recv().await {
                    tx.send(message).await.unwrap();
                }
            });
        }
    });
    for round in 0..400u32 {
        let clients: Vec<_> = (0..32u32)
            .map(|n| {
                let socket = socket.clone();
                tokio::spawn(async move {
                    let stream = tokio::net::UnixStream::connect(&socket)
                        .await
                        .unwrap()
                        .into_std()
                        .unwrap();
                    let (tx, rx) = channel_from_std::<String, String>(stream).unwrap();
                    let message = format!("round {round} client {n}");
                    tx.send(message.clone()).await.unwrap();
                    match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
                        Ok(Ok(echoed)) => assert_eq!(echoed, message),
                        other => panic!("{message} was not echoed: {other:?}"),
                    }
                })
            })
            .collect();
        for client in clients {
            client.await.unwrap();
        }
    }
}
