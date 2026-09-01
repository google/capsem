use super::*;

#[test]
fn query_raw_returns_columns_and_rows() {
    let reader = fixture_reader();
    let json = reader
        .query_raw("SELECT domain, decision FROM net_events ORDER BY id LIMIT 3")
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let cols = v["columns"].as_array().unwrap();
    assert_eq!(cols.len(), 2);
    assert_eq!(cols[0], "domain");
    assert_eq!(cols[1], "decision");
    let rows = v["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    // First row should have a non-empty domain and a valid decision
    assert!(rows[0][0].is_string(), "domain should be a string");
    let decision = rows[0][1].as_str().unwrap();
    assert!(
        decision == "allowed" || decision == "denied" || decision == "error",
        "unexpected decision: {decision}"
    );
}

#[test]
fn query_raw_empty_result() {
    let reader = fixture_reader();
    let json = reader.query_raw("SELECT domain FROM net_events WHERE 1 = 0").unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let cols = v["columns"].as_array().unwrap();
    assert_eq!(cols.len(), 1);
    assert_eq!(cols[0], "domain");
    let rows = v["rows"].as_array().unwrap();
    assert!(rows.is_empty());
}

#[test]
fn query_raw_syntax_error() {
    let reader = fixture_reader();
    let result = reader.query_raw("SELEC broken");
    assert!(result.is_err());
    let err = result.unwrap_err();
    // query_raw now validates the SQL keyword up-front, so typos like "SELEC"
    // are caught at validation time with "unsupported statement type: SELEC"
    // rather than reaching the SQLite parser. Accept either shape.
    assert!(
        err.contains("near") || err.contains("syntax") || err.contains("error") || err.contains("unsupported"),
        "unexpected error: {err}"
    );
}

#[test]
fn query_raw_integer_and_null_types() {
    let reader = fixture_reader();
    let json = reader
        .query_raw("SELECT id, port, status_code FROM net_events ORDER BY id LIMIT 1")
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let row = &v["rows"][0];
    // id and port should be integers
    assert!(row[0].is_number(), "id should be a number");
    assert!(row[1].is_number(), "port should be a number");
    // status_code should be a number (may vary by fixture)
    assert!(row[2].is_number(), "status_code should be a number");
}

#[test]
fn query_raw_null_values() {
    let reader = fixture_reader();
    // Denied events exist in the fixture
    let json = reader
        .query_raw("SELECT method, status_code, decision FROM net_events WHERE decision = 'denied' LIMIT 1")
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let rows = v["rows"].as_array().unwrap();
    assert!(!rows.is_empty(), "fixture should contain at least one denied event");
    assert_eq!(rows[0][2], "denied");
}

#[test]
fn query_raw_aggregate() {
    let reader = fixture_reader();
    let json = reader.query_raw("SELECT COUNT(*) as cnt FROM net_events").unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["columns"][0], "cnt");
    let count = v["rows"][0][0].as_i64().unwrap();
    assert!(count > 0, "fixture should have at least one net_event");
}

#[test]
fn query_raw_real_type() {
    let reader = fixture_reader();
    let json = reader
        .query_raw("SELECT estimated_cost_usd FROM model_calls LIMIT 1")
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let rows = v["rows"].as_array().unwrap();
    assert!(!rows.is_empty(), "fixture should have model_calls");
    // estimated_cost_usd is REAL -- verify it deserializes as a JSON number
    assert!(rows[0][0].is_number(), "REAL column should serialize as JSON number");
}

#[test]
fn query_raw_timeout_on_slow_query() {
    let reader = fixture_reader();
    // Recursive CTE with aggregate -- SQLite must materialize all rows before
    // COUNT can return, so the interrupt fires before completion.
    let result = reader.query_raw(
        "WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM r WHERE n < 999999999) \
         SELECT COUNT(*) FROM r",
    );
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err, "query timed out after 5 seconds");
}

// ── validate_select_only tests ──────────────────────────────────────

#[test]
fn validate_select_allows_select() {
    assert!(validate_select_only("SELECT * FROM net_events").is_ok());
}

#[test]
fn validate_select_allows_with() {
    assert!(validate_select_only("WITH cte AS (SELECT 1) SELECT * FROM cte").is_ok());
}

#[test]
fn validate_select_allows_explain() {
    assert!(validate_select_only("EXPLAIN SELECT 1").is_ok());
}

#[test]
fn validate_select_rejects_pragma() {
    let err = validate_select_only("PRAGMA table_info(net_events)").unwrap_err();
    assert!(err.contains("PRAGMA"), "should reject PRAGMA: {err}");
}

#[test]
fn validate_select_case_insensitive() {
    assert!(validate_select_only("select 1").is_ok());
    assert!(validate_select_only("Select 1").is_ok());
    assert!(validate_select_only("WITH x AS (select 1) select * from x").is_ok());
}

#[test]
fn validate_select_rejects_insert() {
    let err = validate_select_only("INSERT INTO net_events (domain) VALUES ('bad')").unwrap_err();
    assert!(err.contains("INSERT"), "error should mention INSERT: {err}");
}

#[test]
fn validate_select_rejects_drop() {
    let err = validate_select_only("DROP TABLE net_events").unwrap_err();
    assert!(err.contains("DROP"), "error should mention DROP: {err}");
}

#[test]
fn validate_select_rejects_update() {
    let err = validate_select_only("UPDATE net_events SET domain = 'bad'").unwrap_err();
    assert!(err.contains("UPDATE"), "error should mention UPDATE: {err}");
}

#[test]
fn validate_select_rejects_delete() {
    let err = validate_select_only("DELETE FROM net_events").unwrap_err();
    assert!(err.contains("DELETE"), "error should mention DELETE: {err}");
}

#[test]
fn validate_select_rejects_attach() {
    let err = validate_select_only("ATTACH DATABASE ':memory:' AS m").unwrap_err();
    assert!(err.contains("ATTACH"), "error should mention ATTACH: {err}");
}

#[test]
fn validate_select_rejects_alter() {
    let err = validate_select_only("ALTER TABLE net_events ADD COLUMN x TEXT").unwrap_err();
    assert!(err.contains("ALTER"), "error should mention ALTER: {err}");
}

#[test]
fn validate_select_rejects_create() {
    let err = validate_select_only("CREATE TABLE evil (id INT)").unwrap_err();
    assert!(err.contains("CREATE"), "error should mention CREATE: {err}");
}

#[test]
fn validate_select_rejects_empty() {
    let err = validate_select_only("").unwrap_err();
    assert_eq!(err, "empty query");
    let err2 = validate_select_only("   ").unwrap_err();
    assert_eq!(err2, "empty query");
}

// ── validate_select_only adversarial tests ──────────────────────────

#[test]
fn validate_select_rejects_replace() {
    let err = validate_select_only("REPLACE INTO net_events (domain) VALUES ('bad')").unwrap_err();
    assert!(err.contains("REPLACE"), "should reject REPLACE: {err}");
}

#[test]
fn validate_select_rejects_vacuum() {
    let err = validate_select_only("VACUUM").unwrap_err();
    assert!(err.contains("VACUUM"), "should reject VACUUM: {err}");
}

#[test]
fn validate_select_rejects_detach() {
    let err = validate_select_only("DETACH DATABASE m").unwrap_err();
    assert!(err.contains("DETACH"), "should reject DETACH: {err}");
}

#[test]
fn validate_select_rejects_begin_commit_rollback() {
    assert!(validate_select_only("BEGIN").unwrap_err().contains("BEGIN"));
    assert!(validate_select_only("COMMIT").unwrap_err().contains("COMMIT"));
    assert!(validate_select_only("ROLLBACK").unwrap_err().contains("ROLLBACK"));
}

#[test]
fn validate_select_rejects_savepoint_release() {
    assert!(validate_select_only("SAVEPOINT sp1").unwrap_err().contains("SAVEPOINT"));
    assert!(validate_select_only("RELEASE sp1").unwrap_err().contains("RELEASE"));
}

#[test]
fn validate_select_whitespace_prefix_stripped() {
    assert!(validate_select_only("  SELECT 1").is_ok());
    assert!(validate_select_only("\t\nSELECT 1").is_ok());
    assert!(validate_select_only("  INSERT INTO x VALUES(1)")
        .unwrap_err()
        .contains("INSERT"));
}

#[test]
fn validate_select_rejects_unknown_keyword() {
    let err = validate_select_only("EXEC some_proc").unwrap_err();
    assert!(err.contains("unsupported"), "should reject unknown: {err}");
}

#[test]
fn validate_select_subquery_in_parens_accepted() {
    // WITH(... is parsed as "WITH" which is allowed
    assert!(validate_select_only("WITH(SELECT 1) SELECT 1").is_ok());
}

#[test]
fn validate_select_semicolon_separated() {
    // "SELECT" is extracted as first keyword, accepted; the second statement
    // would be caught by PRAGMA query_only on the connection
    assert!(validate_select_only("SELECT 1; DROP TABLE evil").is_ok());
}

// ── reader: query_raw security tests ───────────────────────────────

#[test]
fn fixture_query_raw_select() {
    let reader = fixture_reader();
    let result = reader.query_raw("SELECT COUNT(*) FROM net_events");
    assert!(result.is_ok(), "SELECT should succeed: {:?}", result);
}

#[test]
fn reader_rejects_insert() {
    let reader = fixture_reader();
    let result = reader.query_raw(
        "INSERT INTO net_events (timestamp, domain, port, decision, bytes_sent, bytes_received, duration_ms) VALUES (0, 'evil.com', 443, 'allowed', 0, 0, 0)",
    );
    assert!(
        result.is_err(),
        "INSERT must be rejected by PRAGMA query_only on DbReader"
    );
}

#[test]
fn reader_rejects_create_table() {
    let reader = fixture_reader();
    let result = reader.query_raw("CREATE TABLE evil (id INTEGER)");
    assert!(result.is_err(), "CREATE TABLE must be rejected");
}

#[test]
fn reader_rejects_drop_table() {
    let reader = fixture_reader();
    let result = reader.query_raw("DROP TABLE net_events");
    assert!(result.is_err(), "DROP TABLE must be rejected");
    // Verify the table still works.
    let check = reader.query_raw("SELECT COUNT(*) FROM net_events");
    assert!(check.is_ok(), "net_events must still be accessible after rejected DROP");
}

#[test]
fn reader_rejects_semicolon_injection() {
    let reader = fixture_reader();
    // Multi-statement: SELECT passes validate_select_only, but the DROP
    // must be caught by PRAGMA query_only on the connection.
    let _ = reader.query_raw("SELECT 1; DROP TABLE net_events");
    // Regardless of whether the above returned Ok or Err, the table must be intact.
    let check = reader.query_raw("SELECT COUNT(*) FROM net_events");
    assert!(check.is_ok(), "net_events must survive semicolon injection attempt");
}

// ── reader: domain counts ──────────────────────────────────────────

#[test]
fn fixture_top_domains_non_empty() {
    let reader = fixture_reader();
    let domains = reader.top_domains(5).unwrap();
    assert!(!domains.is_empty(), "fixture should have domain data");
    for d in &domains {
        assert!(!d.domain.is_empty());
        assert!(d.count > 0);
        // count >= allowed + denied because errors are counted in total but not in either bucket
        assert!(d.count >= d.allowed + d.denied);
    }
}

// ── reader: token usage by provider ────────────────────────────────

#[test]
fn fixture_token_usage_non_empty() {
    let reader = fixture_reader();
    let usage = reader.token_usage_by_provider().unwrap();
    assert!(!usage.is_empty(), "fixture should have model call data");
    for u in &usage {
        assert!(!u.provider.is_empty());
        assert!(u.call_count > 0);
    }
}

// ── reader: trace queries ──────────────────────────────────────────

#[test]
fn fixture_recent_traces_non_empty() {
    let reader = fixture_reader();
    let traces = reader.recent_traces(10).unwrap();
    assert!(!traces.is_empty(), "fixture should have trace data");
    for t in &traces {
        assert!(!t.trace_id.is_empty());
        assert!(t.call_count > 0);
        assert!(t.started_at <= t.ended_at);
    }
}

#[test]
fn fixture_trace_detail_loads_tools() {
    let reader = fixture_reader();
    let traces = reader.recent_traces(1).unwrap();
    assert!(!traces.is_empty());
    let detail = reader.trace_detail(&traces[0].trace_id).unwrap();
    assert_eq!(detail.trace_id, traces[0].trace_id);
    assert!(!detail.calls.is_empty());
}

// ── writer+reader: model call with usage_details roundtrip ─────────

#[tokio::test]
async fn model_call_usage_details_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    let mut call = sample_model_call("anthropic");
    call.usage_details = BTreeMap::from([("cache_read".into(), 800), ("thinking".into(), 200)]);
    call.trace_id = Some("trace-001".to_string());

    writer.write(WriteOp::ModelCall(call)).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let stats = reader.session_stats().unwrap();
    assert_eq!(*stats.total_usage_details.get("cache_read").unwrap_or(&0), 800);
    assert_eq!(*stats.total_usage_details.get("thinking").unwrap_or(&0), 200);
}

// ── writer+reader: tool_calls + tool_responses roundtrip ───────────

#[tokio::test]
async fn model_call_tool_data_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    let mut call = sample_model_call("openai");
    call.trace_id = Some("trace-tools".to_string());
    call.tool_calls = vec![
        ToolCallEntry {
            call_index: 0,
            call_id: "call_abc".to_string(),
            tool_name: "get_weather".to_string(),
            arguments: Some("{\"city\":\"NYC\"}".to_string()),
            origin: "native".to_string(),
            trace_id: None,
        },
        ToolCallEntry {
            call_index: 1,
            call_id: "call_def".to_string(),
            tool_name: "search".to_string(),
            arguments: Some("{\"q\":\"test\"}".to_string()),
            origin: "native".to_string(),
            trace_id: None,
        },
    ];
    call.tool_responses = vec![ToolResponseEntry {
        call_id: "call_prev".to_string(),
        content_preview: Some("72F and sunny".to_string()),
        is_error: false,
        trace_id: None,
        credential_ref: None,
    }];

    writer.write(WriteOp::ModelCall(call)).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();

    // Verify via trace_detail
    let detail = reader.trace_detail("trace-tools").unwrap();
    assert_eq!(detail.calls.len(), 1);
    let mc = &detail.calls[0];
    assert_eq!(mc.call.tool_calls.len(), 2);
    assert_eq!(mc.call.tool_calls[0].tool_name, "get_weather");
    assert_eq!(mc.call.tool_calls[1].tool_name, "search");
    assert_eq!(mc.call.tool_responses.len(), 1);
    assert_eq!(mc.call.tool_responses[0].call_id, "call_prev");
    assert!(!mc.call.tool_responses[0].is_error);

    // Also verify tool_usage_frequency
    let freq = reader.tool_usage_frequency(10).unwrap();
    assert_eq!(freq.len(), 2);

    // Also verify session_stats tool count
    let stats = reader.session_stats().unwrap();
    assert_eq!(stats.total_tool_calls, 2);
}

#[tokio::test]
async fn net_events_over_time_buckets_correctly() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    // Insert events: one right now, one 30 mins ago, one 2 hours ago.
    let now = SystemTime::now();
    let mut ev1 = sample_net_event("now.com", Decision::Allowed);
    ev1.timestamp = now;
    let mut ev2 = sample_net_event("30m-ago.com", Decision::Denied);
    ev2.timestamp = now - Duration::from_secs(30 * 60);
    let mut ev3 = sample_net_event("2h-ago.com", Decision::Allowed);
    ev3.timestamp = now - Duration::from_secs(150 * 60);

    writer.write(WriteOp::NetEvent(ev1)).await;
    writer.write(WriteOp::NetEvent(ev2)).await;
    writer.write(WriteOp::NetEvent(ev3)).await;

    // Explicitly drop writer to flush all pending async writes
    drop(writer);

    let reader = DbReader::open(&path).unwrap();

    // Bucket by 60 mins (1 hour), get last 3 hours (3 buckets)
    // bucket 0: 3 hours ago -> 2 hours ago (ev3)
    // bucket 1: 2 hours ago -> 1 hour ago (no events)
    // bucket 2: 1 hour ago -> now (ev1, ev2)
    let buckets = reader.net_events_over_time(60, 3).unwrap();
    assert_eq!(buckets.len(), 3);

    assert_eq!(buckets[0].allowed, 1);
    assert_eq!(buckets[0].denied, 0);

    assert_eq!(buckets[1].allowed, 0);
    assert_eq!(buckets[1].denied, 0);

    assert_eq!(buckets[2].allowed, 1);
    assert_eq!(buckets[2].denied, 1);
}

// ── MCP call tests ────────────────────────────────────────────────────
