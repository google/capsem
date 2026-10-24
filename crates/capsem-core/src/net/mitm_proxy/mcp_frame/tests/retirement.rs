use super::*;

#[tokio::test]
async fn owner_stop_drains_the_original_frame_before_closing_its_ledger() {
    let (endpoint, driver) = gated_endpoint(4, true);
    let directory = tempfile::tempdir().unwrap();
    let db = Arc::new(DbWriter::open(&directory.path().join("session.db"), 64).unwrap());
    let shutdown = tokio_util::sync::CancellationToken::new();
    let (mut guest, server) = tokio::io::duplex(1 << 16);
    let owner = tokio::spawn(serve_io_until_stopped(
        Vec::new(),
        server,
        endpoint,
        Arc::clone(&db),
        shutdown.clone(),
    ));
    let request = br#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"local__slow","arguments":{}}}"#;
    guest
        .write_all(&capsem_proto::encode_mcp_frame(1, 0, "codex", request).unwrap())
        .await
        .unwrap();
    wait_for_started(&driver, 1).await;
    shutdown.cancel();
    assert!(!owner.is_finished(), "stop discarded an admitted original call");
    driver.release.send(true).unwrap();
    let reply = read_reply(&mut guest).await;
    assert!(reply.error.is_none());
    tokio::time::timeout(Duration::from_secs(2), owner)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(driver.started.load(Ordering::SeqCst), 1);
    db.flush_checked().await.unwrap();
    let rows = db
        .reader()
        .unwrap()
        .query_raw("SELECT COUNT(*) FROM tool_calls")
        .unwrap();
    let rows: serde_json::Value = serde_json::from_str(&rows).unwrap();
    assert_eq!(rows["rows"][0][0], 1);
}
