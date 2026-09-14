use super::*;

async fn echo_server() -> SocketAddr {
    let socket = bound("127.0.0.1:0".parse().unwrap()).unwrap();
    let address = socket.local_addr().unwrap();
    tokio::spawn(async move { serve(socket).await });
    address
}

fn client(address: SocketAddr, count: u32, size: usize) -> Args {
    Args {
        address: Some(address.to_string()),
        serve: None,
        count,
        size,
        interval_ms: 1,
        wait_ms: 300,
    }
}

fn sample(document: &serde_json::Value, key: &str) -> f64 {
    document["metrics"][key]["samples"][0].as_f64().unwrap()
}

#[tokio::test]
async fn every_datagram_comes_back_with_its_round_trip() {
    let address = echo_server().await;
    let result = run(client(address, 50, 1400)).await.unwrap();
    assert_eq!(sample(&result, "sent"), 50.0);
    assert_eq!(sample(&result, "received"), 50.0);
    assert_eq!(sample(&result, "lost"), 0.0);
    assert_eq!(
        result["metrics"]["round_trip_ms"]["samples"].as_array().unwrap().len(),
        50
    );
    assert_eq!(result["udp"]["size"], 1400);
}

#[tokio::test]
async fn a_large_datagram_survives_fragmentation_on_loopback() {
    let address = echo_server().await;
    let result = run(client(address, 3, 60_000)).await.unwrap();
    assert_eq!(sample(&result, "received"), 3.0);
}

#[tokio::test]
async fn nobody_answering_is_counted_as_loss_within_the_wait() {
    let unanswered = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let address = unanswered.local_addr().unwrap();
    let result = run(client(address, 5, 64)).await.unwrap();
    assert_eq!(sample(&result, "sent"), 5.0);
    assert_eq!(sample(&result, "received"), 0.0);
    assert_eq!(sample(&result, "lost"), 5.0);
    assert!(
        result["metrics"].get("round_trip_ms").is_none(),
        "no samples, no metric"
    );
}

#[test]
fn a_datagram_carries_its_sequence_first() {
    let bytes = datagram(7, 64);
    assert_eq!(bytes.len(), 64);
    assert_eq!(sequence_of(&bytes), Some(7));
    assert_eq!(sequence_of(&bytes[..3]), None, "too short to carry one");
    assert_eq!(datagram(9, 4).len(), 4, "the sequence alone is the smallest datagram");
}

#[tokio::test]
async fn a_host_name_is_resolved_before_measuring() {
    let address = echo_server().await;
    let mut args = client(address, 3, 64);
    args.address = Some(format!("localhost:{}", address.port()));
    let result = run(args).await.unwrap();
    assert_eq!(sample(&result, "received"), 3.0);
    let mut malformed = client(address, 1, 64);
    malformed.address = Some("no-port-here".into());
    assert!(
        run(malformed).await.is_err(),
        "a target without a port is an error, not a loss"
    );
}
