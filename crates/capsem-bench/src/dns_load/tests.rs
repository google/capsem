use super::*;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn logical_clients_reuse_one_udp_socket_each() {
    let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let target = server.local_addr().unwrap();
    let peers = Arc::new(Mutex::new(HashSet::new()));
    let server_peers = Arc::clone(&peers);
    let responder = tokio::spawn(async move {
        let mut query = [0_u8; 512];
        loop {
            let (length, peer) = server.recv_from(&mut query).await.unwrap();
            server_peers.lock().unwrap().insert(peer);
            let mut response = query[..length].to_vec();
            response[2] |= 0x80;
            response[3] = (response[3] & 0xf0) | 3;
            server.send_to(&response, peer).await.unwrap();
        }
    });

    let concurrency = 16;
    let report = run(Args {
        target,
        qname: "load-test.capsem-bogus".into(),
        qtype: 1,
        concurrency,
        duration_s: 0.25,
        timeout_ms: 1_000,
    })
    .await
    .unwrap();
    responder.abort();

    assert_eq!(report.errors, 0);
    assert!(report.total_requests > concurrency);
    assert_eq!(report.decision_distribution["denied"], report.total_requests);
    assert!(
        peers.lock().unwrap().len() <= concurrency,
        "one logical client opened more than one UDP socket"
    );
}

#[test]
fn servfail_is_a_failed_request_not_a_successful_decision() {
    let report = summarize(
        vec![
            RequestSample {
                status: 2,
                size: 40,
                latency_ms: 2.0,
                attempts: 1,
                error: None,
                required_text_present: false,
                secret_shaped_fixture_seen: false,
            },
            RequestSample {
                status: 3,
                size: 40,
                latency_ms: 3.0,
                attempts: 1,
                error: None,
                required_text_present: false,
                secret_shaped_fixture_seen: false,
            },
        ],
        2,
        Duration::from_secs(1),
        Duration::from_secs(1),
        40,
    );

    assert_eq!(report.errors, 1);
    assert_eq!(report.error_distribution["SERVFAIL"], 1);
    assert_eq!(report.decision_distribution["error"], 1);
    assert_eq!(report.decision_distribution["denied"], 1);
    assert_eq!(report.rps, 1.0);
}
