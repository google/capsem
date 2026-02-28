/// Integration tests for capsem-logger: write+read roundtrips, batching,
/// concurrent writes, shutdown, WAL concurrent access, adversarial inputs,
/// and raw SQL query endpoint.
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use capsem_logger::{
    validate_select_only, DbReader, DbWriter, Decision, ModelCall, NetEvent, ToolCallEntry,
    ToolResponseEntry, WriteOp,
};

/// Open the shared test fixture at data/fixtures/test.db (read-only).
fn fixture_reader() -> DbReader {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop(); // crates/capsem-logger -> crates
    path.pop(); // crates -> repo root
    path.push("data/fixtures/test.db");
    DbReader::open(&path).expect("failed to open fixture test.db")
}

fn sample_net_event(domain: &str, decision: Decision) -> NetEvent {
    NetEvent {
        timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(1700000000),
        domain: domain.to_string(),
        port: 443,
        decision,
        process_name: None,
        pid: None,
        method: None,
        path: None,
        query: None,
        status_code: None,
        bytes_sent: 1024,
        bytes_received: 4096,
        duration_ms: 150,
        matched_rule: Some("test".to_string()),
        request_headers: None,
        response_headers: None,
        request_body_preview: None,
        response_body_preview: None,
        conn_type: None,
    }
}

fn http_net_event(domain: &str) -> NetEvent {
    NetEvent {
        timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(1700000000),
        domain: domain.to_string(),
        port: 443,
        decision: Decision::Allowed,
        process_name: Some("curl".to_string()),
        pid: Some(42),
        method: Some("GET".to_string()),
        path: Some("/api/v1/repos".to_string()),
        query: Some("page=1".to_string()),
        status_code: Some(200),
        bytes_sent: 2048,
        bytes_received: 8192,
        duration_ms: 250,
        matched_rule: None,
        request_headers: Some("Host: github.com\r\nUser-Agent: curl".to_string()),
        response_headers: Some("Content-Type: application/json".to_string()),
        request_body_preview: None,
        response_body_preview: Some("{\"repos\":[]}".to_string()),
        conn_type: Some("https".to_string()),
    }
}

fn sample_model_call(provider: &str) -> ModelCall {
    ModelCall {
        timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(1700000000),
        provider: provider.to_string(),
        model: Some("claude-sonnet-4-20250514".to_string()),
        process_name: Some("claude".to_string()),
        pid: Some(1234),
        method: "POST".to_string(),
        path: "/v1/messages".to_string(),
        stream: true,
        system_prompt_preview: Some("You are helpful.".to_string()),
        messages_count: 3,
        tools_count: 2,
        request_bytes: 2048,
        request_body_preview: Some("{\"model\":\"...\"}".to_string()),
        message_id: Some("msg_01".to_string()),
        status_code: Some(200),
        text_content: Some("Hello world!".to_string()),
        thinking_content: None,
        stop_reason: Some("end_turn".to_string()),
        input_tokens: Some(25),
        output_tokens: Some(10),
        duration_ms: 1500,
        response_bytes: 4096,
        estimated_cost_usd: 0.001,
        trace_id: None,
        tool_calls: vec![
            ToolCallEntry {
                call_index: 0,
                call_id: "toolu_01".to_string(),
                tool_name: "get_weather".to_string(),
                arguments: Some("{\"city\":\"NYC\"}".to_string()),
            },
        ],
        tool_responses: vec![
            ToolResponseEntry {
                call_id: "toolu_prev".to_string(),
                content_preview: Some("72F and sunny".to_string()),
                is_error: false,
            },
        ],
    }
}

// ── File-backed write+read roundtrips ────────────────────────────────

#[tokio::test]
async fn net_event_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    writer.write(WriteOp::NetEvent(http_net_event("github.com"))).await;
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

// ── Count queries ────────────────────────────────────────────────────

#[tokio::test]
async fn net_event_counts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    for _ in 0..3 {
        writer.write(WriteOp::NetEvent(sample_net_event("a.com", Decision::Allowed))).await;
    }
    for _ in 0..2 {
        writer.write(WriteOp::NetEvent(sample_net_event("b.com", Decision::Denied))).await;
    }
    writer.write(WriteOp::NetEvent(sample_net_event("c.com", Decision::Error))).await;
    drop(writer);

    let reader = capsem_logger::DbReader::open(&path).unwrap();
    let (total, allowed, denied) = reader.net_event_counts().unwrap();
    assert_eq!(total, 6);
    assert_eq!(allowed, 3);
    assert_eq!(denied, 2);
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
    assert_eq!(reader.net_event_counts().unwrap(), (0, 0, 0));
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
            writer.write(WriteOp::NetEvent(sample_net_event(&format!("site{i}.com"), Decision::Allowed))).await;
        }
        // Drop flushes all pending writes.
    }

    let reader = capsem_logger::DbReader::open(&path).unwrap();
    assert_eq!(reader.net_event_counts().unwrap().0, 10);
}

// ── Concurrent writes ────────────────────────────────────────────────

#[tokio::test]
async fn concurrent_async_writes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 256).unwrap();

    for i in 0..50 {
        let ok = writer.try_write(WriteOp::NetEvent(
            sample_net_event(&format!("concurrent{i}.com"), Decision::Allowed),
        ));
        assert!(ok, "try_write should succeed with large channel");
    }
    drop(writer);

    let reader = capsem_logger::DbReader::open(&path).unwrap();
    assert_eq!(reader.net_event_counts().unwrap().0, 50);
}

// ── WAL concurrent access ───────────────────────────────────────────

#[tokio::test]
async fn reader_works_while_writer_active() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    // Write some events.
    for i in 0..5 {
        writer.write(WriteOp::NetEvent(sample_net_event(&format!("wal{i}.com"), Decision::Allowed))).await;
    }

    // Give writer thread a moment to flush.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Open a reader while writer is still alive.
    let reader = writer.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert_eq!(events.len(), 5);

    // Write more events and read again.
    writer.write(WriteOp::NetEvent(sample_net_event("wal5.com", Decision::Denied))).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

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
        conn_type: Some("".to_string()),
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
        timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(1700000000),
        provider: "anthropic".to_string(),
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
        message_id: None,
        status_code: Some(200),
        text_content: Some("Bonjour le monde!".to_string()),
        thinking_content: None,
        stop_reason: Some("end_turn".to_string()),
        input_tokens: Some(5),
        output_tokens: Some(3),
        duration_ms: 100,
        response_bytes: 50,
        estimated_cost_usd: 0.0,
        trace_id: None,
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
        writer.write(WriteOp::NetEvent(
            sample_net_event(&format!("rapid{i}.com"), Decision::Allowed),
        )).await;
    }
    drop(writer);

    let reader = capsem_logger::DbReader::open(&path).unwrap();
    assert_eq!(reader.net_event_counts().unwrap().0, 500);
}

// ── Mixed operations ─────────────────────────────────────────────────

#[tokio::test]
async fn mixed_net_events_and_model_calls() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    writer.write(WriteOp::NetEvent(sample_net_event("net1.com", Decision::Allowed))).await;
    writer.write(WriteOp::ModelCall(sample_model_call("anthropic"))).await;
    writer.write(WriteOp::NetEvent(sample_net_event("net2.com", Decision::Denied))).await;
    writer.write(WriteOp::ModelCall(sample_model_call("openai"))).await;
    drop(writer);

    let reader = capsem_logger::DbReader::open(&path).unwrap();
    assert_eq!(reader.net_event_counts().unwrap().0, 2);
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
    call.tool_calls = (0..10).map(|i| ToolCallEntry {
        call_index: i,
        call_id: format!("toolu_{i:02}"),
        tool_name: format!("tool_{i}"),
        arguments: Some(format!("{{\"arg\":{i}}}")),
    }).collect();
    call.tool_responses = (0..5).map(|i| ToolResponseEntry {
        call_id: format!("toolu_{i:02}"),
        content_preview: Some(format!("result {i}")),
        is_error: i == 3,
    }).collect();
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
        writer.write(WriteOp::NetEvent(sample_net_event("persist.com", Decision::Allowed))).await;
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
    writer.write(WriteOp::NetEvent(sample_net_event("deep.com", Decision::Allowed))).await;
    drop(writer);

    assert!(path.exists());
    let reader = capsem_logger::DbReader::open(&path).unwrap();
    assert_eq!(reader.net_event_counts().unwrap().0, 1);
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

    writer.write(WriteOp::NetEvent(sample_net_event("a.com", Decision::Allowed))).await;
    writer.write(WriteOp::NetEvent(sample_net_event("b.com", Decision::Denied))).await;
    writer.write(WriteOp::NetEvent(sample_net_event("c.com", Decision::Error))).await;
    writer.write(WriteOp::NetEvent(sample_net_event("d.com", Decision::Error))).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let (total, allowed, denied) = reader.net_event_counts().unwrap();
    assert_eq!(total, 4);
    assert_eq!(allowed, 1);
    assert_eq!(denied, 1);
    // Error events are in total but not in allowed or denied.
    let error_count = total - allowed - denied;
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
    }];
    call1.tool_responses = Vec::new();

    let mut call2 = sample_model_call("openai");
    call2.tool_calls = vec![ToolCallEntry {
        call_index: 0,
        call_id: "tc_second".to_string(),
        tool_name: "tool_b".to_string(),
        arguments: None,
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

    writer.write(WriteOp::NetEvent(sample_net_event("live.com", Decision::Allowed))).await;
    // Give writer thread time to flush.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let reader = writer.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1, "reader from writer.reader() should see written data");
    assert_eq!(events[0].domain, "live.com");
}

// ── Session stats + new query methods ───────────────────────────────

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

    writer.write(WriteOp::NetEvent(sample_net_event("a.com", Decision::Allowed))).await;
    writer.write(WriteOp::NetEvent(sample_net_event("b.com", Decision::Denied))).await;
    writer.write(WriteOp::NetEvent(sample_net_event("c.com", Decision::Error))).await;
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
        writer.write(WriteOp::NetEvent(sample_net_event("a.com", Decision::Allowed))).await;
    }
    writer.write(WriteOp::NetEvent(sample_net_event("b.com", Decision::Denied))).await;
    for _ in 0..2 {
        writer.write(WriteOp::NetEvent(sample_net_event("c.com", Decision::Allowed))).await;
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
        writer.write(WriteOp::NetEvent(sample_net_event(&format!("d{i}.com"), Decision::Allowed))).await;
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
        ToolCallEntry { call_index: 0, call_id: "t1".into(), tool_name: "read_file".into(), arguments: None },
        ToolCallEntry { call_index: 1, call_id: "t2".into(), tool_name: "write_file".into(), arguments: None },
    ];
    writer.write(WriteOp::ModelCall(call)).await;

    let mut call2 = sample_model_call("anthropic");
    call2.tool_calls = vec![
        ToolCallEntry { call_index: 0, call_id: "t3".into(), tool_name: "read_file".into(), arguments: None },
    ];
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
    let json = reader
        .query_raw("SELECT domain FROM net_events WHERE 1 = 0")
        .unwrap();
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
    assert!(
        err.contains("near") || err.contains("syntax") || err.contains("error"),
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
    let json = reader
        .query_raw("SELECT COUNT(*) as cnt FROM net_events")
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["columns"][0], "cnt");
    let count = v["rows"][0][0].as_i64().unwrap();
    assert!(count > 0, "fixture should have at least one net_event");
}

#[test]
fn query_raw_real_type() {
    let reader = fixture_reader();
    let json = reader
        .query_raw("SELECT estimated_cost_usd FROM model_calls WHERE estimated_cost_usd > 0 LIMIT 1")
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let val = v["rows"][0][0].as_f64().unwrap();
    assert!(val > 0.0, "expected positive cost, got {val}");
}

#[test]
fn query_raw_timeout_on_slow_query() {
    let reader = fixture_reader();
    // Recursive CTE with aggregate -- SQLite must materialize all rows before
    // COUNT can return, so the interrupt fires before completion.
    let result = reader.query_raw(
        "WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM r WHERE n < 999999999) \
         SELECT COUNT(*) FROM r"
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
fn validate_select_allows_pragma() {
    assert!(validate_select_only("PRAGMA table_info(net_events)").is_ok());
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
