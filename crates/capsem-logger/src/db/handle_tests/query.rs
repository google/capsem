use super::*;

#[tokio::test]
async fn db_handle_query_binds_params_and_caps_rows() {
    let p = temp_db_path("query-binds-params-caps-rows");
    {
        let db = DbHandle::open(&p).expect("open handle");
        db.ready().await.expect("db ready");
    }
    {
        let mut conn = rusqlite::Connection::open(&p).expect("open query fixture");
        let tx = conn.transaction().expect("start fixture transaction");
        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO net_events (timestamp, domain, decision)
                     VALUES (?1, ?2, 'allowed')",
                )
                .expect("prepare fixture insert");
            for i in 0..10_050 {
                stmt.execute(("2026-01-01T00:00:00Z", format!("bind-{i:05}.example")))
                    .expect("insert fixture row");
            }
        }
        tx.commit().expect("commit fixture rows");
    }

    let db = DbHandle::open(&p).expect("reopen handle");
    let raw = db
        .query(
            "SELECT domain, decision FROM net_events
             WHERE decision = ? AND domain LIKE ?
             ORDER BY domain",
            &[json!("allowed"), json!("bind-%.example")],
        )
        .await
        .expect("query should bind params on DB-owned worker");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("query JSON");
    let rows = value["rows"].as_array().expect("rows array");

    assert_eq!(
        value["columns"],
        json!(["domain", "decision"]),
        "query(sql, params) must return deterministic columns. {DB_BOUNDARY_RATIONALE}"
    );
    assert_eq!(
        rows.len(),
        10_000,
        "query(sql, params) must cap route-visible output at the DB boundary. {DB_BOUNDARY_RATIONALE}"
    );
    assert_eq!(rows.first(), Some(&json!(["bind-00000.example", "allowed"])));
    assert_eq!(rows.last(), Some(&json!(["bind-09999.example", "allowed"])));
}

#[tokio::test]
async fn db_handle_query_returns_exact_columns_rows() {
    let p = temp_db_path("query-exact-columns-rows");
    let db = DbHandle::open(&p).expect("open handle");
    db.ready().await.expect("db ready");
    db.write(WriteOp::NetEvent(make_net_event("exact.example", Decision::Denied)))
        .await
        .expect("write exact fixture");
    db.flush_for_tests().await;

    let raw = db
        .query(
            "SELECT domain, port, decision, method, path, bytes_sent, bytes_received
             FROM net_events WHERE domain = ?",
            &[json!("exact.example")],
        )
        .await
        .expect("query exact rows");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("query JSON");

    assert_eq!(
        value["columns"],
        json!([
            "domain",
            "port",
            "decision",
            "method",
            "path",
            "bytes_sent",
            "bytes_received"
        ]),
        "DbHandle::query must preserve exact column order. {DB_BOUNDARY_RATIONALE}"
    );
    assert_eq!(
        value["rows"],
        json!([["exact.example", 443, "denied", "GET", "/api", 11, 22]]),
        "DbHandle::query must preserve exact row values. {DB_BOUNDARY_RATIONALE}"
    );
}

#[tokio::test]
async fn db_handle_query_records_read_metrics() {
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};

    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _guard = metrics::set_default_local_recorder(&recorder);

    let p = temp_db_path("query-records-read-metrics");
    let db = DbHandle::open(&p).expect("open handle");
    db.ready().await.expect("db ready");
    db.write(WriteOp::NetEvent(make_net_event("metrics.example", Decision::Allowed)))
        .await
        .expect("write metric fixture");
    db.flush_for_tests().await;

    let raw = db
        .query(
            "SELECT domain, decision FROM net_events WHERE domain = ?",
            &[json!("metrics.example")],
        )
        .await
        .expect("query metric rows");
    assert!(
        raw.contains("metrics.example"),
        "metric test must exercise a real DB query"
    );

    let snapshot = snapshotter.snapshot().into_vec();
    assert!(snapshot
        .iter()
        .any(|(key, _, _, value)| { key.key().name() == DB_QUERY_TOTAL && matches!(value, DebugValue::Counter(_)) }));
    assert!(snapshot.iter().any(|(key, _, _, value)| {
        key.key().name() == DB_QUERY_DURATION_MS && matches!(value, DebugValue::Histogram(_))
    }));
    assert!(snapshot.iter().any(|(key, _, _, value)| {
        key.key().name() == DB_QUERY_RESULT_ROWS && matches!(value, DebugValue::Histogram(_))
    }));
    assert!(snapshot.iter().any(|(key, _, _, value)| {
        key.key().name() == DB_QUERY_RESULT_BYTES && matches!(value, DebugValue::Histogram(_))
    }));
    assert!(snapshot.iter().any(|(key, _, _, value)| {
        key.key().name() == DB_QUERY_PARAMS_COUNT && matches!(value, DebugValue::Histogram(_))
    }));
}

#[tokio::test]
async fn db_handle_query_many_preserves_order_and_exact_rows() {
    let p = temp_db_path("query-many-preserves-order");
    let db = DbHandle::open(&p).expect("open handle");
    db.ready().await.expect("db ready");
    db.write(WriteOp::NetEvent(make_net_event(
        "batch-one.example",
        Decision::Allowed,
    )))
    .await
    .expect("write first fixture");
    db.write(WriteOp::NetEvent(make_net_event("batch-two.example", Decision::Denied)))
        .await
        .expect("write second fixture");
    db.flush_for_tests().await;

    let raw = db
        .query_many(vec![
            (
                "SELECT decision FROM net_events WHERE domain = ?".to_string(),
                vec![json!("batch-one.example")],
            ),
            (
                "SELECT domain FROM net_events WHERE decision = ? ORDER BY domain".to_string(),
                vec![json!("denied")],
            ),
        ])
        .await
        .expect("query batch rows");

    assert_eq!(raw.len(), 2);
    let first: serde_json::Value = serde_json::from_str(&raw[0]).expect("first query JSON");
    let second: serde_json::Value = serde_json::from_str(&raw[1]).expect("second query JSON");
    assert_eq!(first["columns"], json!(["decision"]));
    assert_eq!(first["rows"], json!([["allowed"]]));
    assert_eq!(second["columns"], json!(["domain"]));
    assert_eq!(second["rows"], json!([["batch-two.example"]]));
}

#[tokio::test]
async fn db_handle_query_many_rejects_mutations() {
    let p = temp_db_path("query-many-rejects-mutations");
    let db = DbHandle::open(&p).expect("open handle");
    db.ready().await.expect("db ready");

    let error = db
        .query_many(vec![
            ("SELECT COUNT(*) AS count FROM net_events".to_string(), vec![]),
            ("DELETE FROM net_events WHERE domain = ?".to_string(), vec![json!("x")]),
        ])
        .await
        .expect_err("query_many must reject mutation SQL");
    assert!(
        error.contains("DELETE") && error.contains("not allowed"),
        "DbHandle::query_many must reject mutations before SQLite execution: {error}. {DB_BOUNDARY_RATIONALE}"
    );
}

#[tokio::test]
async fn db_handle_query_many_cache_invalidates_after_write() {
    let p = temp_db_path("query-many-cache-invalidates");
    let db = DbHandle::open(&p).expect("open handle");
    db.ready().await.expect("db ready");
    db.write(WriteOp::NetEvent(make_net_event(
        "cache-one.example",
        Decision::Allowed,
    )))
    .await
    .expect("write first fixture");
    db.flush_for_tests().await;

    let count_query = || vec![("SELECT COUNT(*) AS count FROM net_events".to_string(), Vec::new())];
    let first = db.query_many(count_query()).await.expect("first cached batch query");
    let first: serde_json::Value = serde_json::from_str(&first[0]).expect("first count JSON");
    assert_eq!(first["rows"], json!([[1]]));

    db.write(WriteOp::NetEvent(make_net_event(
        "cache-two.example",
        Decision::Allowed,
    )))
    .await
    .expect("write second fixture");
    db.flush_for_tests().await;

    let second = db
        .query_many(count_query())
        .await
        .expect("second cached batch query after invalidation");
    let second: serde_json::Value = serde_json::from_str(&second[0]).expect("second count JSON");
    assert_eq!(
        second["rows"],
        json!([[2]]),
        "DbHandle-owned query_many cache must invalidate when the DB handle accepts writes. {DB_BOUNDARY_RATIONALE}"
    );
}

#[tokio::test]
async fn external_reader_query_many_does_not_cache_across_external_writes() {
    let p = temp_db_path("external-query-many-no-stale-cache");
    let writer = DbWriter::open(&p, 16).expect("open owning writer");
    writer.write_blocking(WriteOp::NetEvent(make_net_event(
        "external-cache-one.example",
        Decision::Allowed,
    )));
    writer.flush().await;

    let db = DbHandle::open_external_reader(&p).expect("open service external reader");
    db.ready().await.expect("external reader ready");
    let count_query = || vec![("SELECT COUNT(*) AS count FROM net_events".to_string(), Vec::new())];

    let first = db.query_many(count_query()).await.expect("first external batch query");
    let first: serde_json::Value = serde_json::from_str(&first[0]).expect("first count JSON");
    assert_eq!(first["rows"], json!([[1]]));

    writer.write_blocking(WriteOp::NetEvent(make_net_event(
        "external-cache-two.example",
        Decision::Allowed,
    )));
    writer.flush().await;

    let second = db
        .query_many(count_query())
        .await
        .expect("second external batch query after external write");
    let second: serde_json::Value = serde_json::from_str(&second[0]).expect("second count JSON");
    assert_eq!(
        second["rows"],
        json!([[2]]),
        "external-reader query_many must resync from disk on every call instead of returning a stale batch cache. {DB_BOUNDARY_RATIONALE}"
    );
}

#[tokio::test]
async fn db_handle_query_rejects_mutations() {
    let p = temp_db_path("query-rejects-mutations");
    let db = DbHandle::open(&p).expect("open handle");
    db.ready().await.expect("db ready");

    let error = db
        .query("DELETE FROM net_events WHERE domain = ?", &[json!("x")])
        .await
        .expect_err("query must reject mutation SQL");
    assert!(
        error.contains("DELETE") && error.contains("not allowed"),
        "DbHandle::query must reject mutations before SQLite execution: {error}. {DB_BOUNDARY_RATIONALE}"
    );
}

#[tokio::test]
async fn db_handle_query_uses_worker_not_runtime_block() {
    let p = temp_db_path("query-worker-not-runtime-block");
    let db = DbHandle::open(&p).expect("open handle");
    db.ready().await.expect("db ready");
    let ticker = tokio::spawn(async {
        let mut ticks = 0;
        for _ in 0..100 {
            tokio::task::yield_now().await;
            ticks += 1;
        }
        ticks
    });

    let (query, ticks) = tokio::join!(db.query("SELECT COUNT(*) AS count FROM net_events", &[]), ticker);

    query.expect("query should complete through DB worker");
    assert_eq!(
        ticks.expect("ticker task should complete"),
        100,
        "DbHandle::query must await a DB-owned worker instead of blocking the tokio runtime. {DB_BOUNDARY_RATIONALE}"
    );
}
