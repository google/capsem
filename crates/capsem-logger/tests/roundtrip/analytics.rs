use super::*;

#[tokio::test]
async fn ledger_counters_empty_db() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();
    drop(writer);

    // A fresh ledger carries a snapshot, and it is all zeros: an empty
    // session reads as empty, not as a missing snapshot.
    let counters = ledger_counters(&path).await;
    assert_eq!(counters, capsem_logger::counters::LedgerCounters::default());
}

#[tokio::test]
async fn ledger_counters_with_data() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    writer
        .write(WriteOp::NetEvent(sample_net_event("a.com", Decision::Allowed)))
        .await;
    writer
        .write(WriteOp::NetEvent(sample_net_event("b.com", Decision::Denied)))
        .await;
    writer
        .write(WriteOp::NetEvent(sample_net_event("c.com", Decision::Error)))
        .await;
    writer.write(WriteOp::ModelCall(sample_model_call("anthropic"))).await;
    drop(writer);

    let counters = ledger_counters(&path).await;
    assert_eq!(counters.net.total, 3);
    assert_eq!(counters.net.allowed, 1);
    assert_eq!(counters.net.denied, 1);
    assert_eq!(counters.net.error, 1);
    assert_eq!(counters.net.bytes_sent, 3 * 1024);
    assert_eq!(counters.net.bytes_received, 3 * 4096);
    assert_eq!(counters.model.total.calls, 1);
    assert_eq!(counters.model.total.input_tokens, 25);
    assert_eq!(counters.model.total.output_tokens, 10);
    assert_eq!(counters.tools.calls, 1);
    assert!(counters.model.total.cost_micro_usd > 0);
}

#[tokio::test]
async fn ledger_counters_null_tokens() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    let mut call = sample_model_call("anthropic");
    call.input_tokens = None;
    call.output_tokens = None;
    call.estimated_cost_usd = 0.0;
    writer.write(WriteOp::ModelCall(call)).await;
    drop(writer);

    let counters = ledger_counters(&path).await;
    assert_eq!(counters.model.total.calls, 1);
    assert_eq!(counters.model.total.input_tokens, 0);
    assert_eq!(counters.model.total.output_tokens, 0);
}

#[tokio::test]
async fn search_net_events_by_domain() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    writer.write(WriteOp::NetEvent(http_net_event("github.com"))).await;
    writer.write(WriteOp::NetEvent(http_net_event("pypi.org"))).await;
    writer.write(WriteOp::NetEvent(http_net_event("api.github.com"))).await;
    drop(writer);

    let reader = capsem_logger::DbReader::open(&path).unwrap();
    let results = reader.search_net_events("github", 100).unwrap();
    assert_eq!(results.len(), 2);
}

#[tokio::test]
async fn search_net_events_by_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    writer.write(WriteOp::NetEvent(http_net_event("api.com"))).await;
    drop(writer);

    let reader = capsem_logger::DbReader::open(&path).unwrap();
    let results = reader.search_net_events("repos", 100).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].path.as_deref(), Some("/api/v1/repos"));
}

#[tokio::test]
async fn search_net_events_no_match() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    writer.write(WriteOp::NetEvent(http_net_event("api.com"))).await;
    drop(writer);

    let reader = capsem_logger::DbReader::open(&path).unwrap();
    let results = reader.search_net_events("nonexistent_xyz", 100).unwrap();
    assert!(results.is_empty());
}

#[tokio::test]
async fn search_net_events_sql_injection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    writer.write(WriteOp::NetEvent(http_net_event("safe.com"))).await;
    drop(writer);

    let reader = capsem_logger::DbReader::open(&path).unwrap();
    // Parameterized queries make this safe; should return empty, not crash.
    let results = reader.search_net_events("'; DROP TABLE net_events; --", 100).unwrap();
    assert!(results.is_empty());
    // Table still works:
    let events = reader.recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
}

#[tokio::test]
async fn search_model_calls_by_provider() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    writer.write(WriteOp::ModelCall(sample_model_call("anthropic"))).await;
    let mut google_call = sample_model_call("google");
    google_call.model = Some("gemini-2.0-flash".to_string());
    writer.write(WriteOp::ModelCall(google_call)).await;
    drop(writer);

    let reader = capsem_logger::DbReader::open(&path).unwrap();
    let results = reader.search_model_calls("anthropic", 100).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].1.provider, "anthropic");
}

#[tokio::test]
async fn model_usage_counted_by_provider() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    writer.write(WriteOp::ModelCall(sample_model_call("anthropic"))).await;
    writer.write(WriteOp::ModelCall(sample_model_call("anthropic"))).await;
    let mut google_call = sample_model_call("google");
    google_call.input_tokens = Some(100);
    google_call.output_tokens = Some(50);
    google_call.estimated_cost_usd = 0.005;
    writer.write(WriteOp::ModelCall(google_call)).await;
    drop(writer);

    let counters = ledger_counters(&path).await;
    let usage = &counters.model.by_model;
    assert_eq!(usage.len(), 2, "{usage:?}");

    // Every sample call names the same model, so each provider has one entry.
    let anth = &usage["anthropic"]["claude-sonnet-4-20250514"];
    assert_eq!(anth.calls, 2);
    assert_eq!(anth.input_tokens, 50);
    assert_eq!(anth.output_tokens, 20);
    assert!(anth.cost_micro_usd > 0);

    let goog = &usage["google"]["claude-sonnet-4-20250514"];
    assert_eq!(goog.calls, 1);
    assert_eq!(goog.input_tokens, 100);
    assert_eq!(goog.output_tokens, 50);
    assert_eq!(goog.cost_micro_usd, capsem_logger::counters::cost_micro_usd(0.005));
}

#[tokio::test]
async fn tool_calls_counted_by_tool() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    let mut call = sample_model_call("anthropic");
    call.tool_calls = vec![
        ToolCallEntry {
            event_id: None,
            call_index: 0,
            call_id: "t1".into(),
            tool_name: "read_file".into(),
            arguments: None,
            origin: "native".into(),
            trace_id: None,
        },
        ToolCallEntry {
            event_id: None,
            call_index: 1,
            call_id: "t2".into(),
            tool_name: "write_file".into(),
            arguments: None,
            origin: "native".into(),
            trace_id: None,
        },
    ];
    writer.write(WriteOp::ModelCall(call)).await;

    let mut call2 = sample_model_call("anthropic");
    call2.tool_calls = vec![ToolCallEntry {
        event_id: None,
        call_index: 0,
        call_id: "t3".into(),
        tool_name: "read_file".into(),
        arguments: None,
        origin: "native".into(),
        trace_id: None,
    }];
    writer.write(WriteOp::ModelCall(call2)).await;
    drop(writer);

    let counters = ledger_counters(&path).await;
    let freq = &counters.tools.by_tool;
    assert_eq!(freq.len(), 2, "{freq:?}");
    assert_eq!(freq["read_file"].calls, 2);
    assert_eq!(freq["write_file"].calls, 1);
    assert_eq!(counters.tools.calls, 3);
}

#[tokio::test]
async fn estimated_cost_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    let mut call = sample_model_call("anthropic");
    call.estimated_cost_usd = 0.0042;
    writer.write(WriteOp::ModelCall(call)).await;
    drop(writer);

    let reader = capsem_logger::DbReader::open(&path).unwrap();
    let calls = reader.recent_model_calls(1).unwrap();
    assert_eq!(calls.len(), 1);
    assert!((calls[0].1.estimated_cost_usd - 0.0042).abs() < 1e-10);

    let counters = ledger_counters(&path).await;
    assert_eq!(counters.model.total.cost_micro_usd, 4_200);
}

// ── Trace ID ─────────────────────────────────────────────────────────

#[tokio::test]
async fn trace_id_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    let mut call = sample_model_call("anthropic");
    call.trace_id = Some("trace_abc123".to_string());
    writer.write(WriteOp::ModelCall(call)).await;

    let mut call2 = sample_model_call("openai");
    call2.trace_id = None;
    writer.write(WriteOp::ModelCall(call2)).await;
    drop(writer);

    let reader = capsem_logger::DbReader::open(&path).unwrap();
    let calls = reader.recent_model_calls(10).unwrap();
    assert_eq!(calls.len(), 2);

    // Most recent first (openai with no trace_id)
    assert!(calls[0].1.trace_id.is_none());
    // Older (anthropic with trace_id)
    assert_eq!(calls[1].1.trace_id.as_deref(), Some("trace_abc123"));
}

// ========================================================================
// query_raw + validate_select_only tests (fixture-based)
// ========================================================================
