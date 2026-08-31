use super::*;

#[tokio::test]
async fn session_stats_empty_db() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();
    drop(writer);

    let reader = capsem_logger::DbReader::open(&path).unwrap();
    let stats = reader.session_stats().unwrap();
    assert_eq!(stats.net_total, 0);
    assert_eq!(stats.net_allowed, 0);
    assert_eq!(stats.net_denied, 0);
    assert_eq!(stats.net_error, 0);
    assert_eq!(stats.net_bytes_sent, 0);
    assert_eq!(stats.net_bytes_received, 0);
    assert_eq!(stats.model_call_count, 0);
    assert_eq!(stats.total_input_tokens, 0);
    assert_eq!(stats.total_output_tokens, 0);
    assert_eq!(stats.total_tool_calls, 0);
    assert_eq!(stats.total_estimated_cost_usd, 0.0);
}

#[tokio::test]
async fn session_stats_with_data() {
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

    let reader = capsem_logger::DbReader::open(&path).unwrap();
    let stats = reader.session_stats().unwrap();
    assert_eq!(stats.net_total, 3);
    assert_eq!(stats.net_allowed, 1);
    assert_eq!(stats.net_denied, 1);
    assert_eq!(stats.net_error, 1);
    assert_eq!(stats.net_bytes_sent, 3 * 1024);
    assert_eq!(stats.net_bytes_received, 3 * 4096);
    assert_eq!(stats.model_call_count, 1);
    assert_eq!(stats.total_input_tokens, 25);
    assert_eq!(stats.total_output_tokens, 10);
    assert_eq!(stats.total_tool_calls, 1);
    assert!(stats.total_estimated_cost_usd > 0.0);
}

#[tokio::test]
async fn session_stats_null_tokens() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    let mut call = sample_model_call("anthropic");
    call.input_tokens = None;
    call.output_tokens = None;
    call.estimated_cost_usd = 0.0;
    writer.write(WriteOp::ModelCall(call)).await;
    drop(writer);

    let reader = capsem_logger::DbReader::open(&path).unwrap();
    let stats = reader.session_stats().unwrap();
    assert_eq!(stats.total_input_tokens, 0);
    assert_eq!(stats.total_output_tokens, 0);
}

#[tokio::test]
async fn top_domains_ordering() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    // 3 events for a.com, 1 for b.com, 2 for c.com
    for _ in 0..3 {
        writer
            .write(WriteOp::NetEvent(sample_net_event("a.com", Decision::Allowed)))
            .await;
    }
    writer
        .write(WriteOp::NetEvent(sample_net_event("b.com", Decision::Denied)))
        .await;
    for _ in 0..2 {
        writer
            .write(WriteOp::NetEvent(sample_net_event("c.com", Decision::Allowed)))
            .await;
    }
    drop(writer);

    let reader = capsem_logger::DbReader::open(&path).unwrap();
    let top = reader.top_domains(10).unwrap();
    assert_eq!(top.len(), 3);
    assert_eq!(top[0].domain, "a.com");
    assert_eq!(top[0].count, 3);
    assert_eq!(top[1].domain, "c.com");
    assert_eq!(top[1].count, 2);
    assert_eq!(top[2].domain, "b.com");
    assert_eq!(top[2].count, 1);
    assert_eq!(top[2].denied, 1);
}

#[tokio::test]
async fn top_domains_limit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    for i in 0..5 {
        writer
            .write(WriteOp::NetEvent(sample_net_event(
                &format!("d{i}.com"),
                Decision::Allowed,
            )))
            .await;
    }
    drop(writer);

    let reader = capsem_logger::DbReader::open(&path).unwrap();
    let top = reader.top_domains(3).unwrap();
    assert_eq!(top.len(), 3);
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
async fn token_usage_by_provider() {
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

    let reader = capsem_logger::DbReader::open(&path).unwrap();
    let usage = reader.token_usage_by_provider().unwrap();
    assert_eq!(usage.len(), 2);

    // anthropic has 2 calls, should be first (ordered by count DESC)
    let anth = usage.iter().find(|u| u.provider == "anthropic").unwrap();
    assert_eq!(anth.call_count, 2);
    assert_eq!(anth.total_input_tokens, 50);
    assert_eq!(anth.total_output_tokens, 20);
    assert!(anth.total_estimated_cost_usd > 0.0);

    let goog = usage.iter().find(|u| u.provider == "google").unwrap();
    assert_eq!(goog.call_count, 1);
    assert_eq!(goog.total_input_tokens, 100);
    assert_eq!(goog.total_output_tokens, 50);
    assert_eq!(goog.total_estimated_cost_usd, 0.005);
}

#[tokio::test]
async fn tool_usage_frequency() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    let mut call = sample_model_call("anthropic");
    call.tool_calls = vec![
        ToolCallEntry {
            call_index: 0,
            call_id: "t1".into(),
            tool_name: "read_file".into(),
            arguments: None,
            origin: "native".into(),
            trace_id: None,
        },
        ToolCallEntry {
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
        call_index: 0,
        call_id: "t3".into(),
        tool_name: "read_file".into(),
        arguments: None,
        origin: "native".into(),
        trace_id: None,
    }];
    writer.write(WriteOp::ModelCall(call2)).await;
    drop(writer);

    let reader = capsem_logger::DbReader::open(&path).unwrap();
    let freq = reader.tool_usage_frequency(10).unwrap();
    assert_eq!(freq.len(), 2);
    assert_eq!(freq[0].tool_name, "read_file");
    assert_eq!(freq[0].count, 2);
    assert_eq!(freq[1].tool_name, "write_file");
    assert_eq!(freq[1].count, 1);
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

    let stats = reader.session_stats().unwrap();
    assert!((stats.total_estimated_cost_usd - 0.0042).abs() < 1e-10);
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

#[tokio::test]
async fn recent_traces_groups_by_trace_id() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    // 3 calls in trace_A, 2 in trace_B
    for i in 0..3 {
        let mut call = sample_model_call("anthropic");
        call.trace_id = Some("trace_A".to_string());
        call.input_tokens = Some(10);
        call.output_tokens = Some(5);
        call.timestamp = SystemTime::UNIX_EPOCH + Duration::from_secs(1700000000 + i);
        writer.write(WriteOp::ModelCall(call)).await;
    }
    for i in 0..2 {
        let mut call = sample_model_call("openai");
        call.trace_id = Some("trace_B".to_string());
        call.input_tokens = Some(20);
        call.output_tokens = Some(10);
        call.timestamp = SystemTime::UNIX_EPOCH + Duration::from_secs(1700000010 + i);
        writer.write(WriteOp::ModelCall(call)).await;
    }
    drop(writer);

    let reader = capsem_logger::DbReader::open(&path).unwrap();
    let traces = reader.recent_traces(10).unwrap();
    assert_eq!(traces.len(), 2);

    // Most recent trace first (trace_B has higher max id)
    assert_eq!(traces[0].trace_id, "trace_B");
    assert_eq!(traces[0].call_count, 2);
    assert_eq!(traces[0].total_input_tokens, 40);
    assert_eq!(traces[0].total_output_tokens, 20);
    assert_eq!(traces[0].provider, "openai");

    assert_eq!(traces[1].trace_id, "trace_A");
    assert_eq!(traces[1].call_count, 3);
    assert_eq!(traces[1].total_input_tokens, 30);
}

#[tokio::test]
async fn trace_detail_loads_tool_data() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    let mut call = sample_model_call("anthropic");
    call.trace_id = Some("trace_X".to_string());
    writer.write(WriteOp::ModelCall(call)).await;
    drop(writer);

    let reader = capsem_logger::DbReader::open(&path).unwrap();
    let detail = reader.trace_detail("trace_X").unwrap();
    assert_eq!(detail.trace_id, "trace_X");
    assert_eq!(detail.calls.len(), 1);
    assert!(!detail.calls[0].call.tool_calls.is_empty());
    assert!(!detail.calls[0].call.tool_responses.is_empty());
}

#[tokio::test]
async fn traces_without_trace_id_excluded() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    let mut call_with = sample_model_call("anthropic");
    call_with.trace_id = Some("trace_Y".to_string());
    writer.write(WriteOp::ModelCall(call_with)).await;

    let mut call_without = sample_model_call("openai");
    call_without.trace_id = None;
    writer.write(WriteOp::ModelCall(call_without)).await;
    drop(writer);

    let reader = capsem_logger::DbReader::open(&path).unwrap();
    let traces = reader.recent_traces(10).unwrap();
    assert_eq!(traces.len(), 1);
    assert_eq!(traces[0].trace_id, "trace_Y");
}

#[tokio::test]
async fn trace_ordering_newest_first() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    // Write trace_old first, then trace_new
    let mut old = sample_model_call("anthropic");
    old.trace_id = Some("trace_old".to_string());
    writer.write(WriteOp::ModelCall(old)).await;

    let mut new = sample_model_call("openai");
    new.trace_id = Some("trace_new".to_string());
    writer.write(WriteOp::ModelCall(new)).await;
    drop(writer);

    let reader = capsem_logger::DbReader::open(&path).unwrap();
    let traces = reader.recent_traces(10).unwrap();
    assert_eq!(traces[0].trace_id, "trace_new");
    assert_eq!(traces[1].trace_id, "trace_old");
}

// ========================================================================
// query_raw + validate_select_only tests (fixture-based)
// ========================================================================
