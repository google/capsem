use super::*;

async fn server() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { serve(listener, 64 * 1024).await });
    address
}

fn client(address: SocketAddr, direction: Direction, streams: usize) -> Args {
    Args {
        address: Some(address),
        serve: None,
        direction,
        streams,
        seconds: 1,
        chunk_bytes: 64 * 1024,
    }
}

fn sample(document: &serde_json::Value, key: &str) -> f64 {
    document["metrics"][key]["samples"][0].as_f64().unwrap()
}

#[tokio::test]
async fn upload_moves_bytes_one_way() {
    let address = server().await;
    let result = run(client(address, Direction::Upload, 2)).await.unwrap();
    assert!(sample(&result, "bytes_sent") > 0.0);
    assert_eq!(sample(&result, "bytes_received"), 0.0);
    assert!(sample(&result, "send_megabits_per_sec") > 0.0);
    assert_eq!(result["throughput"]["direction"], "upload");
    assert_eq!(sample(&result, "streams"), 2.0);
}

#[tokio::test]
async fn download_moves_bytes_the_other_way() {
    let address = server().await;
    let result = run(client(address, Direction::Download, 1)).await.unwrap();
    assert_eq!(sample(&result, "bytes_sent"), 0.0);
    assert!(sample(&result, "bytes_received") > 0.0);
}

#[tokio::test]
async fn bidirectional_moves_bytes_both_ways() {
    let address = server().await;
    let result = run(client(address, Direction::Bidirectional, 4)).await.unwrap();
    assert!(sample(&result, "bytes_sent") > 0.0);
    assert!(sample(&result, "bytes_received") > 0.0);
    assert!(sample(&result, "elapsed_seconds") >= 1.0);
}

#[tokio::test]
async fn latency_records_one_sample_per_round_trip() {
    let address = server().await;
    let result = run(client(address, Direction::Latency, 2)).await.unwrap();
    let samples = result["metrics"]["round_trip_ms"]["samples"].as_array().unwrap();
    assert!(samples.len() >= 2, "at least one echo per stream: {samples:?}");
    assert!(samples.iter().all(|s| s.as_f64().unwrap() > 0.0));
    assert_eq!(
        sample(&result, "bytes_sent"),
        (samples.len() * ECHO_BYTES) as f64,
        "every echo is accounted for"
    );
}

#[tokio::test]
async fn unknown_direction_byte_closes_only_that_connection() {
    let address = server().await;
    let mut bad = TcpStream::connect(address).await.unwrap();
    bad.write_u8(9).await.unwrap();
    let mut buffer = [0u8; 1];
    assert_eq!(
        bad.read(&mut buffer).await.unwrap(),
        0,
        "server must close on a bad header"
    );
    let result = run(client(address, Direction::Upload, 1)).await.unwrap();
    assert!(sample(&result, "bytes_sent") > 0.0, "the server keeps serving others");
}

#[tokio::test]
async fn server_that_closes_a_download_early_is_a_failed_run() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let _ = stream.read_u8().await;
        drop(stream);
    });
    assert!(run(client(address, Direction::Download, 1)).await.is_err());
}

#[test]
fn direction_bytes_round_trip_and_reject_the_rest() {
    for direction in [
        Direction::Upload,
        Direction::Download,
        Direction::Bidirectional,
        Direction::Latency,
    ] {
        assert_eq!(Direction::from_wire(direction.wire()), Some(direction));
    }
    assert_eq!(Direction::from_wire(4), None);
    assert_eq!(Direction::from_wire(255), None);
}

#[tokio::test]
async fn bounds_are_enforced_before_any_connection() {
    let address = "127.0.0.1:1".parse().unwrap();
    let mut args = client(address, Direction::Upload, 0);
    assert!(run(args.clone()).await.is_err());
    args.streams = 1;
    args.seconds = 0;
    assert!(run(args.clone()).await.is_err());
    args.seconds = 1;
    args.chunk_bytes = 1;
    assert!(run(args).await.is_err());
}
