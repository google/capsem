use super::*;

#[tokio::test]
async fn net_event_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    let credential_ref = credential_reference("github", "github_pat_roundtrip");
    let mut event = http_net_event("github.com");
    event.credential_ref = Some(credential_ref.clone());
    writer.write(WriteOp::NetEvent(event)).await;
    drop(writer); // flush

    let reader = capsem_logger::DbReader::open(&path).unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let e = &events[0];
    assert_eq!(e.domain, "github.com");
    assert_eq!(e.decision, Decision::Allowed);
    assert_eq!(e.method.as_deref(), Some("GET"));
    assert_eq!(e.path.as_deref(), Some("/api/v1/repos"));
    assert_eq!(e.query.as_deref(), Some("page=1"));
    assert_eq!(e.status_code, Some(200));
    assert_eq!(e.bytes_sent, 2048);
    assert_eq!(e.bytes_received, 8192);
    assert_eq!(e.process_name.as_deref(), Some("curl"));
    assert_eq!(e.pid, Some(42));
    assert_eq!(e.conn_type.as_deref(), Some("https"));
    assert_eq!(e.credential_ref.as_deref(), Some(credential_ref.as_str()));
}

#[tokio::test]
async fn model_call_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    writer.write(WriteOp::ModelCall(sample_model_call("anthropic"))).await;
    drop(writer);

    let reader = capsem_logger::DbReader::open(&path).unwrap();
    let calls = reader.recent_model_calls(10).unwrap();
    assert_eq!(calls.len(), 1);
    let (id, c) = &calls[0];
    assert!(*id > 0);
    assert_eq!(c.provider, "anthropic");
    assert_eq!(c.protocol.as_deref(), Some("anthropic"));
    assert_eq!(c.model.as_deref(), Some("claude-sonnet-4-20250514"));
    assert_eq!(c.method, "POST");
    assert_eq!(c.path, "/v1/messages");
    assert!(c.stream);
    assert_eq!(c.messages_count, 3);
    assert_eq!(c.tools_count, 2);
    assert_eq!(c.message_id.as_deref(), Some("msg_01"));
    assert_eq!(c.status_code, Some(200));
    assert_eq!(c.text_content.as_deref(), Some("Hello world!"));
    assert_eq!(c.stop_reason.as_deref(), Some("end_turn"));
    assert_eq!(c.input_tokens, Some(25));
    assert_eq!(c.output_tokens, Some(10));
    assert_eq!(c.process_name.as_deref(), Some("claude"));
    assert_eq!(c.pid, Some(1234));

    // Verify tool calls
    let tcs = reader.tool_calls_for(*id).unwrap();
    assert_eq!(tcs.len(), 1);
    assert_eq!(tcs[0].call_id, "toolu_01");
    assert_eq!(tcs[0].tool_name, "get_weather");
    assert_eq!(tcs[0].arguments.as_deref(), Some("{\"city\":\"NYC\"}"));

    // Verify tool responses
    let trs = reader.tool_responses_for(*id).unwrap();
    assert_eq!(trs.len(), 1);
    assert_eq!(trs[0].call_id, "toolu_prev");
    assert_eq!(trs[0].content_preview.as_deref(), Some("72F and sunny"));
    assert!(!trs[0].is_error);
}

#[tokio::test]
async fn model_items_dedup_by_trace_kind_hash_and_call_id_across_restarts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");

    let mut call = sample_model_call("openai");
    call.trace_id = Some("trace_ironbank_dedup".to_string());
    call.model = Some("gemma4:latest".to_string());
    call.path = "/v1/responses".to_string();
    call.request_body_preview =
        Some(r#"{"model":"gemma4:latest","input":"write nonce","tools":[{"name":"exec_command"}]}"#.to_string());
    call.thinking_content = Some("dedup reasoning".to_string());
    call.text_content = Some("dedup response".to_string());
    call.tool_calls = vec![ToolCallEntry {
        call_index: 0,
        call_id: "call_dedup_01".to_string(),
        tool_name: "exec_command".to_string(),
        arguments: Some(r#"{"cmd":"printf nonce > /root/dedup.txt"}"#.to_string()),
        origin: "native".to_string(),
        trace_id: None,
    }];
    call.tool_responses = Vec::new();

    {
        let writer = DbWriter::open(&path, 64).unwrap();
        writer.write(WriteOp::ModelCall(call.clone())).await;
        writer.write(WriteOp::ModelCall(call.clone())).await;
        drop(writer);
    }

    let mut response_call = call.clone();
    response_call.request_body_preview = Some(
        r#"{"input":[{"type":"function_call_output","call_id":"call_dedup_01","output":"Process exited with code 0"}]}"#
            .to_string(),
    );
    response_call.thinking_content = None;
    response_call.text_content = None;
    response_call.tool_calls = Vec::new();
    response_call.tool_responses = vec![ToolResponseEntry {
        call_id: "call_dedup_01".to_string(),
        content_preview: Some("Process exited with code 0".to_string()),
        is_error: false,
        trace_id: None,
        credential_ref: None,
    }];

    {
        let writer = DbWriter::open(&path, 64).unwrap();
        writer.write(WriteOp::ModelCall(response_call.clone())).await;
        writer.write(WriteOp::ModelCall(response_call)).await;
        drop(writer);
    }

    let conn = rusqlite::Connection::open(&path).unwrap();
    let rows = conn
        .prepare(
            "SELECT kind, call_id, tool_name, arguments, content, content_hash
             FROM model_items
             WHERE trace_id = 'trace_ironbank_dedup'
             ORDER BY kind",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(rows.len(), 5, "{rows:#?}");
    let kinds: Vec<_> = rows.iter().map(|row| row.0.as_str()).collect();
    assert_eq!(
        kinds,
        ["reasoning", "request", "response", "tool_call", "tool_response"]
    );
    assert!(rows.iter().all(|row| row.5.len() == 71 && row.5.starts_with("blake3:")));
    assert!(rows.iter().any(|row| row.1 == "call_dedup_01"
        && row.2.as_deref() == Some("exec_command")
        && row.3.as_deref() == Some(r#"{"cmd":"printf nonce > /root/dedup.txt"}"#)));
    assert!(rows
        .iter()
        .any(|row| row.1 == "call_dedup_01" && row.4.as_deref() == Some("Process exited with code 0")));
}

#[tokio::test]
async fn model_items_without_trace_id_dedup_across_restarts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let call = sample_model_call("anthropic");

    {
        let writer = DbWriter::open(&path, 64).unwrap();
        writer.write(WriteOp::ModelCall(call.clone())).await;
        writer.flush().await;
    }

    let first_count: i64 = rusqlite::Connection::open(&path)
        .unwrap()
        .query_row("SELECT count(*) FROM model_items", [], |row| row.get(0))
        .unwrap();
    assert!(first_count > 0);

    {
        let writer = DbWriter::open(&path, 64).unwrap();
        writer.write(WriteOp::ModelCall(call)).await;
        writer.flush().await;
    }

    let final_count: i64 = rusqlite::Connection::open(&path)
        .unwrap()
        .query_row("SELECT count(*) FROM model_items", [], |row| row.get(0))
        .unwrap();
    assert_eq!(final_count, first_count);
}

// ── Count queries ────────────────────────────────────────────────────

#[tokio::test]
async fn net_event_counts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    for _ in 0..3 {
        writer
            .write(WriteOp::NetEvent(sample_net_event("a.com", Decision::Allowed)))
            .await;
    }
    for _ in 0..2 {
        writer
            .write(WriteOp::NetEvent(sample_net_event("b.com", Decision::Denied)))
            .await;
    }
    writer
        .write(WriteOp::NetEvent(sample_net_event("c.com", Decision::Error)))
        .await;
    drop(writer);

    let reader = capsem_logger::DbReader::open(&path).unwrap();
    let counts = reader.net_event_counts().unwrap();
    assert_eq!(counts.total, 6);
    assert_eq!(counts.allowed, 3);
    assert_eq!(counts.denied, 2);
}

#[tokio::test]
async fn model_call_count() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    for _ in 0..5 {
        writer.write(WriteOp::ModelCall(sample_model_call("openai"))).await;
    }
    drop(writer);

    let reader = capsem_logger::DbReader::open(&path).unwrap();
    assert_eq!(reader.model_call_count().unwrap(), 5);
}

// ── Ordering ─────────────────────────────────────────────────────────

#[tokio::test]
async fn recent_returns_newest_first() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    for i in 0..5 {
        let mut event = sample_net_event(&format!("site{i}.com"), Decision::Allowed);
        event.timestamp = SystemTime::UNIX_EPOCH + Duration::from_secs(1700000000 + i);
        writer.write(WriteOp::NetEvent(event)).await;
    }
    drop(writer);

    let reader = capsem_logger::DbReader::open(&path).unwrap();
    let events = reader.recent_net_events(3).unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].domain, "site4.com");
    assert_eq!(events[1].domain, "site3.com");
    assert_eq!(events[2].domain, "site2.com");
}

// ── Empty DB ─────────────────────────────────────────────────────────

#[tokio::test]
async fn empty_db_queries() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();
    drop(writer);

    let reader = capsem_logger::DbReader::open(&path).unwrap();
    assert!(reader.recent_net_events(10).unwrap().is_empty());
    assert!(reader.recent_model_calls(10).unwrap().is_empty());
    let empty = reader.net_event_counts().unwrap();
    assert_eq!((empty.total, empty.allowed, empty.denied), (0, 0, 0));
    assert_eq!(reader.model_call_count().unwrap(), 0);
    assert!(reader.tool_calls_for(999).unwrap().is_empty());
}

// ── Writer shutdown ──────────────────────────────────────────────────

#[tokio::test]
async fn writer_drop_flushes_pending_writes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");

    {
        let writer = DbWriter::open(&path, 256).unwrap();
        for i in 0..10 {
            writer
                .write(WriteOp::NetEvent(sample_net_event(
                    &format!("site{i}.com"),
                    Decision::Allowed,
                )))
                .await;
        }
        // Drop flushes all pending writes.
    }

    let reader = capsem_logger::DbReader::open(&path).unwrap();
    assert_eq!(reader.net_event_counts().unwrap().total, 10);
}

// ── Concurrent writes ────────────────────────────────────────────────

#[tokio::test]
async fn concurrent_async_writes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 256).unwrap();

    for i in 0..50 {
        let ok = writer.try_write(WriteOp::NetEvent(sample_net_event(
            &format!("concurrent{i}.com"),
            Decision::Allowed,
        )));
        assert!(ok, "try_write should succeed with large channel");
    }
    drop(writer);

    let reader = capsem_logger::DbReader::open(&path).unwrap();
    assert_eq!(reader.net_event_counts().unwrap().total, 50);
}

// ── WAL concurrent access ───────────────────────────────────────────

#[tokio::test]
async fn reader_works_while_writer_active() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    // Write some events.
    for i in 0..5 {
        writer
            .write(WriteOp::NetEvent(sample_net_event(
                &format!("wal{i}.com"),
                Decision::Allowed,
            )))
            .await;
    }

    // `DbWriter::write` is an enqueue path. Use the DB-owned flush barrier
    // before asserting that a standalone reader observes the committed ledger.
    writer.flush().await;

    // Open a reader while writer is still alive.
    let reader = writer.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert_eq!(events.len(), 5);

    // Write more events and read again.
    writer
        .write(WriteOp::NetEvent(sample_net_event("wal5.com", Decision::Denied)))
        .await;
    writer.flush().await;

    let events = reader.recent_net_events(10).unwrap();
    assert_eq!(events.len(), 6);

    drop(writer);
}

// ── Adversarial inputs ──────────────────────────────────────────────

#[tokio::test]
async fn empty_strings() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    let event = NetEvent {
        event_id: None,
        timestamp: SystemTime::UNIX_EPOCH,
        domain: "".to_string(),
        port: 0,
        decision: Decision::Error,
        process_name: Some("".to_string()),
        pid: None,
        method: Some("".to_string()),
        path: Some("".to_string()),
        query: Some("".to_string()),
        status_code: None,
        bytes_sent: 0,
        bytes_received: 0,
        duration_ms: 0,
        matched_rule: Some("".to_string()),
        request_headers: Some("".to_string()),
        response_headers: Some("".to_string()),
        request_body_preview: Some("".to_string()),
        response_body_preview: Some("".to_string()),
        request_body_full: None,
        response_body_full: None,
        conn_type: Some("".to_string()),
        policy_mode: None,
        policy_action: None,
        policy_rule: None,
        policy_reason: None,
        trace_id: None,
        credential_ref: None,
    };

    writer.write(WriteOp::NetEvent(event)).await;
    drop(writer);

    let reader = capsem_logger::DbReader::open(&path).unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].domain, "");
}

#[tokio::test]
async fn unicode_strings() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    let event = sample_net_event("xn--n3h.example.com", Decision::Allowed);
    writer.write(WriteOp::NetEvent(event)).await;

    let call = ModelCall {
        event_id: None,
        timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(1700000000),
        provider: "anthropic".to_string(),
        protocol: Some("anthropic".to_string()),
        model: Some("claude".to_string()),
        process_name: None,
        pid: None,
        method: "POST".to_string(),
        path: "/v1/messages".to_string(),
        stream: false,
        system_prompt_preview: None,
        messages_count: 1,
        tools_count: 0,
        request_bytes: 100,
        request_body_preview: None,
        request_body_full: None,
        message_id: None,
        status_code: Some(200),
        text_content: Some("Bonjour le monde!".to_string()),
        thinking_content: None,
        response_body_full: Some("Bonjour le monde!".to_string()),
        stop_reason: Some("end_turn".to_string()),
        input_tokens: Some(5),
        output_tokens: Some(3),
        usage_details: std::collections::BTreeMap::new(),
        duration_ms: 100,
        response_bytes: 50,
        estimated_cost_usd: 0.0,
        trace_id: None,
        credential_ref: None,
        tool_calls: Vec::new(),
        tool_responses: Vec::new(),
    };
    writer.write(WriteOp::ModelCall(call)).await;
    drop(writer);

    let reader = capsem_logger::DbReader::open(&path).unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert_eq!(events[0].domain, "xn--n3h.example.com");

    let calls = reader.recent_model_calls(10).unwrap();
    assert_eq!(calls[0].1.text_content.as_deref(), Some("Bonjour le monde!"));
}

#[tokio::test]
async fn large_body_previews() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    let large_body = "x".repeat(100_000);
    let mut event = sample_net_event("big.com", Decision::Allowed);
    event.request_body_preview = Some(large_body.clone());
    event.response_body_preview = Some(large_body.clone());

    writer.write(WriteOp::NetEvent(event)).await;
    drop(writer);

    let reader = capsem_logger::DbReader::open(&path).unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert_eq!(events[0].request_body_preview.as_ref().unwrap().len(), 100_000);
}

// ── Rapid-fire writes ────────────────────────────────────────────────

#[tokio::test]
async fn rapid_fire_writes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 1024).unwrap();

    for i in 0..500 {
        writer
            .write(WriteOp::NetEvent(sample_net_event(
                &format!("rapid{i}.com"),
                Decision::Allowed,
            )))
            .await;
    }
    drop(writer);

    let reader = capsem_logger::DbReader::open(&path).unwrap();
    assert_eq!(reader.net_event_counts().unwrap().total, 500);
}

// ── Mixed operations ─────────────────────────────────────────────────

#[tokio::test]
async fn mixed_net_events_and_model_calls() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    writer
        .write(WriteOp::NetEvent(sample_net_event("net1.com", Decision::Allowed)))
        .await;
    writer.write(WriteOp::ModelCall(sample_model_call("anthropic"))).await;
    writer
        .write(WriteOp::NetEvent(sample_net_event("net2.com", Decision::Denied)))
        .await;
    writer.write(WriteOp::ModelCall(sample_model_call("openai"))).await;
    drop(writer);

    let reader = capsem_logger::DbReader::open(&path).unwrap();
    assert_eq!(reader.net_event_counts().unwrap().total, 2);
    assert_eq!(reader.model_call_count().unwrap(), 2);
}

// ── Model call with no tools ─────────────────────────────────────────

#[tokio::test]
async fn model_call_no_tools() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    let mut call = sample_model_call("openai");
    call.tool_calls = Vec::new();
    call.tool_responses = Vec::new();
    writer.write(WriteOp::ModelCall(call)).await;
    drop(writer);

    let reader = capsem_logger::DbReader::open(&path).unwrap();
    let calls = reader.recent_model_calls(10).unwrap();
    assert_eq!(calls.len(), 1);
    let tcs = reader.tool_calls_for(calls[0].0).unwrap();
    assert!(tcs.is_empty());
    let trs = reader.tool_responses_for(calls[0].0).unwrap();
    assert!(trs.is_empty());
}

// ── Model call with many tools ───────────────────────────────────────

#[tokio::test]
async fn model_call_many_tools() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    let mut call = sample_model_call("anthropic");
    call.tool_calls = (0..10)
        .map(|i| ToolCallEntry {
            call_index: i,
            call_id: format!("toolu_{i:02}"),
            tool_name: format!("tool_{i}"),
            arguments: Some(format!("{{\"arg\":{i}}}")),
            origin: "native".to_string(),
            trace_id: None,
        })
        .collect();
    call.tool_responses = (0..5)
        .map(|i| ToolResponseEntry {
            call_id: format!("toolu_{i:02}"),
            content_preview: Some(format!("result {i}")),
            is_error: i == 3,
            trace_id: None,
            credential_ref: None,
        })
        .collect();
    writer.write(WriteOp::ModelCall(call)).await;
    drop(writer);

    let reader = capsem_logger::DbReader::open(&path).unwrap();
    let calls = reader.recent_model_calls(10).unwrap();
    let id = calls[0].0;

    let tcs = reader.tool_calls_for(id).unwrap();
    assert_eq!(tcs.len(), 10);
    assert_eq!(tcs[0].call_id, "toolu_00");
    assert_eq!(tcs[9].call_id, "toolu_09");

    let trs = reader.tool_responses_for(id).unwrap();
    assert_eq!(trs.len(), 5);
    assert!(trs[3].is_error);
    assert!(!trs[0].is_error);
}

// ── DB file persistence ──────────────────────────────────────────────

#[tokio::test]
async fn db_file_persists_across_opens() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");

    // First open: write data.
    {
        let writer = DbWriter::open(&path, 64).unwrap();
        writer
            .write(WriteOp::NetEvent(sample_net_event("persist.com", Decision::Allowed)))
            .await;
        drop(writer);
    }

    // Second open: data still there.
    {
        let writer = DbWriter::open(&path, 64).unwrap();
        let reader = writer.reader().unwrap();
        let events = reader.recent_net_events(10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].domain, "persist.com");
        drop(writer);
    }
}

// ── Parent directory creation ────────────────────────────────────────

#[tokio::test]
async fn creates_parent_directories() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("deep").join("nested").join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();
    writer
        .write(WriteOp::NetEvent(sample_net_event("deep.com", Decision::Allowed)))
        .await;
    drop(writer);

    assert!(path.exists());
    let reader = capsem_logger::DbReader::open(&path).unwrap();
    assert_eq!(reader.net_event_counts().unwrap().total, 1);
}

// ========================================================================
// Audit-driven tests: these test expected behavior identified by the
// capsem-logger audit. Written before fixes (TDD red phase).
// ========================================================================

// ── CRITICAL: DbWriter::reader() on in-memory writer is a silent trap ──

/// reader() on an in-memory DbWriter should return Err, not silently
/// create an isolated empty database that can never see the writer's data.
#[test]
fn writer_reader_on_in_memory_returns_error() {
    let writer = DbWriter::open_in_memory(64).unwrap();
    assert!(
        writer.reader().is_err(),
        "reader() on in-memory writer must return Err, not a disconnected empty DB"
    );
}

// ── HIGH: Body preview size cap enforcement ─────────────────────────────

/// The logger should enforce a maximum size on body preview fields to
/// prevent unbounded storage from adversarial or buggy callers.
#[tokio::test]
async fn net_event_body_preview_capped() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    let huge = "x".repeat(500_000); // 500KB -- well beyond any reasonable preview
    let mut event = sample_net_event("big.com", Decision::Allowed);
    event.request_body_preview = Some(huge.clone());
    event.response_body_preview = Some(huge);

    writer.write(WriteOp::NetEvent(event)).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let events = reader.recent_net_events(10).unwrap();
    let req_preview = events[0].request_body_preview.as_ref().unwrap();
    let resp_preview = events[0].response_body_preview.as_ref().unwrap();
    assert!(
        req_preview.len() <= 262_144,
        "request_body_preview should be capped at 256KB, got {}",
        req_preview.len()
    );
    assert!(
        resp_preview.len() <= 262_144,
        "response_body_preview should be capped at 256KB, got {}",
        resp_preview.len()
    );
}

/// Model call text_content and thinking_content should also be capped.
#[tokio::test]
async fn model_call_content_fields_capped() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    let huge = "y".repeat(500_000);
    let mut call = sample_model_call("anthropic");
    call.text_content = Some(huge.clone());
    call.thinking_content = Some(huge);
    call.request_body_preview = Some("z".repeat(500_000));

    writer.write(WriteOp::ModelCall(call)).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let calls = reader.recent_model_calls(10).unwrap();
    let c = &calls[0].1;
    assert!(
        c.text_content.as_ref().unwrap().len() <= 262_144,
        "text_content should be capped at 256KB"
    );
    assert!(
        c.thinking_content.as_ref().unwrap().len() <= 262_144,
        "thinking_content should be capped at 256KB"
    );
}

// ── MEDIUM: net_event_counts error events explicitly counted ────────────

/// Error events must be counted in total but not in allowed or denied.
/// This makes the arithmetic relationship explicit.
#[tokio::test]
async fn net_event_counts_error_counted_in_total_only() {
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
    writer
        .write(WriteOp::NetEvent(sample_net_event("d.com", Decision::Error)))
        .await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let counts = reader.net_event_counts().unwrap();
    assert_eq!(counts.total, 4);
    assert_eq!(counts.allowed, 1);
    assert_eq!(counts.denied, 1);
    // Error events are in total but not in allowed or denied.
    let error_count = counts.total - counts.allowed - counts.denied;
    assert_eq!(error_count, 2, "error events must be counted in total only");
}

// ── MEDIUM: Multiple model calls get distinct row IDs ───────────────────

/// Two sequential model call inserts must produce distinct row IDs so
/// tool_calls and tool_responses are linked to the correct parent.
#[tokio::test]
async fn multiple_model_calls_get_distinct_ids() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    let mut call1 = sample_model_call("anthropic");
    call1.tool_calls = vec![ToolCallEntry {
        call_index: 0,
        call_id: "tc_first".to_string(),
        tool_name: "tool_a".to_string(),
        arguments: None,
        origin: "native".to_string(),
        trace_id: None,
    }];
    call1.tool_responses = Vec::new();

    let mut call2 = sample_model_call("openai");
    call2.tool_calls = vec![ToolCallEntry {
        call_index: 0,
        call_id: "tc_second".to_string(),
        tool_name: "tool_b".to_string(),
        arguments: None,
        origin: "native".to_string(),
        trace_id: None,
    }];
    call2.tool_responses = Vec::new();

    writer.write(WriteOp::ModelCall(call1)).await;
    writer.write(WriteOp::ModelCall(call2)).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let calls = reader.recent_model_calls(10).unwrap();
    assert_eq!(calls.len(), 2);

    let (id1, _) = &calls[1]; // older (anthropic)
    let (id2, _) = &calls[0]; // newer (openai)
    assert_ne!(id1, id2, "model calls must have distinct row IDs");

    // Verify tool calls are linked to the correct parent.
    let tcs1 = reader.tool_calls_for(*id1).unwrap();
    assert_eq!(tcs1.len(), 1);
    assert_eq!(tcs1[0].tool_name, "tool_a");

    let tcs2 = reader.tool_calls_for(*id2).unwrap();
    assert_eq!(tcs2.len(), 1);
    assert_eq!(tcs2[0].tool_name, "tool_b");
}

// ── MEDIUM: WAL concurrent reader via writer.reader() ───────────────────

/// writer.reader() on a file-backed DB must return a working reader
/// that can see data written through the writer.
#[tokio::test]
async fn writer_reader_on_file_backed_sees_data() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = Arc::new(DbWriter::open(&path, 64).unwrap());

    writer
        .write(WriteOp::NetEvent(sample_net_event("live.com", Decision::Allowed)))
        .await;
    // writer.reader() opens a file-backed reader, so use the explicit DB
    // flush barrier before asserting disk-visible rows.
    writer.flush().await;

    let reader = writer.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1, "reader from writer.reader() should see written data");
    assert_eq!(events[0].domain, "live.com");
}

// ── Session stats + new query methods ───────────────────────────────
