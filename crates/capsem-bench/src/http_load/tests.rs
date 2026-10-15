use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[test]
fn summary_separates_connection_setup_from_warm_requests() {
    let report = summarize(
        vec![
            Sample {
                latency_ms: 10.0,
                bytes: 24,
                error: None,
                cold: true,
            },
            Sample {
                latency_ms: 20.0,
                bytes: 24,
                error: None,
                cold: true,
            },
            Sample {
                latency_ms: 1.0,
                bytes: 24,
                error: None,
                cold: false,
            },
            Sample {
                latency_ms: 2.0,
                bytes: 24,
                error: None,
                cold: false,
            },
            Sample {
                latency_ms: 4.0,
                bytes: 0,
                error: Some("HTTP 502".to_string()),
                cold: false,
            },
        ],
        2,
        Duration::from_secs(1),
        Duration::from_secs(2),
    );

    assert_eq!(report.total_requests, 5);
    assert_eq!(report.errors, 1);
    assert_eq!(report.error_ratio, 0.2);
    assert_eq!(report.rps, 2.0);
    assert_eq!(report.cold_p50_ms, 15.0);
    assert_eq!(report.warm_p50_ms, 1.5);
    assert_eq!(report.response_body_bytes, 96);
    assert_eq!(report.error_distribution["HTTP 502"], 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn logical_clients_do_not_multiply_keepalive_connections() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target = format!("http://{}/tiny", listener.local_addr().unwrap());
    let accepted = Arc::new(AtomicUsize::new(0));
    let accepted_for_server = Arc::clone(&accepted);
    let server = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            accepted_for_server.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                let mut request = Vec::new();
                loop {
                    let mut chunk = [0_u8; 1024];
                    let read = stream.read(&mut chunk).await.unwrap();
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&chunk[..read]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        stream
                            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\nx")
                            .await
                            .unwrap();
                        request.clear();
                    }
                }
            });
        }
    });

    let concurrency = 128;
    let report = run(Args {
        target,
        concurrency,
        duration_s: 0.5,
        timeout_ms: 5_000,
    })
    .await
    .unwrap();
    server.abort();

    assert_eq!(report.errors, 0);
    assert!(report.total_requests > concurrency);
    assert!(
        accepted.load(Ordering::SeqCst) <= concurrency,
        "one logical client opened more than one live keepalive connection"
    );
}
