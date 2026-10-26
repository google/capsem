use super::*;
use crate::{schema, store};

async fn command(arguments: &[&str]) -> anyhow::Result<()> {
    let cli = Cli::try_parse_from(std::iter::once("capsem-bench-rs").chain(arguments.iter().copied()))?;
    execute(cli).await
}

#[tokio::test]
async fn collector_records_are_reported_compared_and_rejected_when_regressed() {
    let root = tempfile::tempdir().unwrap();
    let collectors = root.path().join("collectors");
    std::fs::create_dir(&collectors).unwrap();
    let collector = collectors.join("routes");
    let baseline = root.path().join("baseline.db");
    let current = root.path().join("current.db");
    for (destination, samples) in [(&baseline, "1,1,1"), (&current, "2,2,2")] {
        std::fs::write(&collector, format!(
            "printf '%s\\n' '{{\"metrics\":{{\"cpu_s\":{{\"unit\":\"seconds\",\"samples\":[{samples}]}}}}}}'\n"
        )).unwrap();
        command(&[
            "run", "routes", "--collectors", collectors.to_str().unwrap(),
            "--out", destination.to_str().unwrap(), "--interpreter", "/bin/sh",
            "--channel", "stable", "--commit", "measured-source",
        ]).await.unwrap();
        let db = store::open(destination).unwrap();
        let record = store::latest(&db, schema::Dimension::Routes, std::env::consts::ARCH)
            .unwrap().unwrap();
        assert_eq!(record.release.channel, "stable");
        assert_eq!(record.release.commit, "measured-source");
        assert_eq!(record.metrics[0].key, "routes.cpu_s");
        assert_eq!(record.metrics[0].summary.n, 3);
    }
    let baseline = baseline.to_str().unwrap();
    let current = current.to_str().unwrap();
    command(&["report", "--store", current]).await.unwrap();
    command(&["compare", baseline, current, "routes"]).await.unwrap();
    let error = command(&["verify", "--records", current, "--evidence", baseline]).await.unwrap_err();
    assert!(error.to_string().contains("regressed"), "{error}");
    command(&["verify", "--records", baseline, "--evidence", baseline]).await.unwrap();
    assert!(command(&["compare", baseline, current, "routes", "--runs", "0"]).await.is_err());
    assert!(command(&["run", "unknown-dimension"]).await.is_err());
    command(&["list"]).await.unwrap();
}

#[tokio::test]
async fn protocol_command_records_the_measured_http_samples_and_provenance() {
    let root = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0; 4096];
        let count = socket.read(&mut request).await.unwrap();
        assert!(request[..count].starts_with(b"GET /tiny HTTP/1.1\r\n"));
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 24\r\nConnection: close\r\n\r\n123456789012345678901234").await.unwrap();
    });
    let artifact = root.path().join("artifact.json");
    let collector = root.path().join("collector.json");
    let database = root.path().join("records.db");
    command(&[
        "protocol", "--base-url", &base_url, "--requests", "1", "--concurrency", "1",
        "--scenarios", "tiny_http", "--json-out", artifact.to_str().unwrap(),
        "--collector-out", collector.to_str().unwrap(), "--record", database.to_str().unwrap(),
        "--channel", "stable", "--commit", "protocol-source",
    ]).await.unwrap();
    server.await.unwrap();
    let artifact = read_artifact(&artifact).unwrap();
    assert_eq!(artifact.mock_server_protocol.scenarios[0].successful, 1);
    let db = store::open(&database).unwrap();
    let record = store::latest(&db, schema::Dimension::Protocol, std::env::consts::ARCH)
        .unwrap().unwrap();
    assert_eq!(record.release.commit, "protocol-source");
    assert_eq!(record.metrics.len(), 4);
    let latency = record.metric("protocol.host_direct.tiny_http.latency_ms").unwrap();
    assert_eq!(latency.summary.n, 1);
    assert!(latency.summary.median > 0.0);
    let samples: serde_json::Value = serde_json::from_slice(&std::fs::read(collector).unwrap()).unwrap();
    assert!(samples["metrics"].as_object().unwrap().len() >= 4);
}
