use super::*;
use std::time::Duration;

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
