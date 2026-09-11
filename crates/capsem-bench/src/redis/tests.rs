use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn server(reply: &'static [u8]) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut request = [0; 14];
                while socket.read_exact(&mut request).await.is_ok() {
                    assert_eq!(&request, b"*1\r\n$4\r\nPING\r\n");
                    if socket.write_all(reply).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    address
}

#[tokio::test]
async fn every_pipelined_reply_is_validated_and_counted() {
    let address = server(b"+PONG\r\n").await;
    let result = run(Args {
        address,
        requests: 64,
        concurrency: 4,
        pipeline: 8,
        timeout_ms: 2000,
    })
    .await
    .unwrap();
    assert_eq!(result["metrics"]["completed_requests"]["samples"][0], 64);
    assert_eq!(
        result["metrics"]["batch_latency_ms"]["samples"]
            .as_array()
            .unwrap()
            .len(),
        8
    );
}

#[tokio::test]
async fn incorrect_reply_is_a_failed_benchmark() {
    let address = server(b"+NOPE\r\n").await;
    assert!(run(Args {
        address,
        requests: 8,
        concurrency: 2,
        pipeline: 4,
        timeout_ms: 1000
    })
    .await
    .is_err());
}

#[tokio::test]
async fn stalled_peer_is_bounded() {
    let address = server(b"").await;
    assert!(run(Args {
        address,
        requests: 8,
        concurrency: 2,
        pipeline: 4,
        timeout_ms: 50
    })
    .await
    .is_err());
}
