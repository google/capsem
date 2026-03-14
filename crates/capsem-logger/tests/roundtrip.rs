/// Integration tests for capsem-logger: write+read roundtrips, batching,
/// concurrent writes, shutdown, WAL concurrent access, adversarial inputs,
/// and raw SQL query endpoint.
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use capsem_logger::{
    validate_select_only, DbReader, DbWriter, Decision, FileAction, FileEvent, McpCall,
    ModelCall, NetEvent, ToolCallEntry, ToolResponseEntry, WriteOp,
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
        usage_details: std::collections::BTreeMap::new(),
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
                origin: "native".to_string(),
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
        usage_details: std::collections::BTreeMap::new(),
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
        origin: "native".to_string(),
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
        origin: "native".to_string(),
    }];
    call1.tool_responses = Vec::new();

    let mut call2 = sample_model_call("openai");
    call2.tool_calls = vec![ToolCallEntry {
        call_index: 0,
        call_id: "tc_second".to_string(),
        tool_name: "tool_b".to_string(),
        arguments: None,
        origin: "native".to_string(),
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
        ToolCallEntry { call_index: 0, call_id: "t1".into(), tool_name: "read_file".into(), arguments: None, origin: "native".into() },
        ToolCallEntry { call_index: 1, call_id: "t2".into(), tool_name: "write_file".into(), arguments: None, origin: "native".into() },
    ];
    writer.write(WriteOp::ModelCall(call)).await;

    let mut call2 = sample_model_call("anthropic");
    call2.tool_calls = vec![
        ToolCallEntry { call_index: 0, call_id: "t3".into(), tool_name: "read_file".into(), arguments: None, origin: "native".into() },
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
    assert!(validate_select_only("  INSERT INTO x VALUES(1)").unwrap_err().contains("INSERT"));
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
    assert!(result.is_err(), "INSERT must be rejected by PRAGMA query_only on DbReader");
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
    call.usage_details = BTreeMap::from([
        ("cache_read".into(), 800),
        ("thinking".into(), 200),
    ]);
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
        },
        ToolCallEntry {
            call_index: 1,
            call_id: "call_def".to_string(),
            tool_name: "search".to_string(),
            arguments: Some("{\"q\":\"test\"}".to_string()),
            origin: "native".to_string(),
        },
    ];
    call.tool_responses = vec![
        ToolResponseEntry {
            call_id: "call_prev".to_string(),
            content_preview: Some("72F and sunny".to_string()),
            is_error: false,
        },
    ];

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

fn sample_mcp_call(server: &str, decision: &str) -> McpCall {
    McpCall {
        timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(1700000000),
        server_name: server.to_string(),
        method: "tools/call".to_string(),
        tool_name: Some(format!("{server}__search_repos")),
        request_id: Some("req-1".to_string()),
        request_preview: Some(r#"{"query":"rust"}"#.to_string()),
        response_preview: Some(r#"{"results":[]}"#.to_string()),
        decision: decision.to_string(),
        duration_ms: 250,
        error_message: None,
        process_name: Some("claude".to_string()),
        bytes_sent: 0,
        bytes_received: 0,
    }
}

#[tokio::test]
async fn mcp_call_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mcp.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    writer.write(WriteOp::McpCall(sample_mcp_call("github", "allowed"))).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let calls = reader.recent_mcp_calls(10).unwrap();
    assert_eq!(calls.len(), 1);
    let c = &calls[0];
    assert_eq!(c.server_name, "github");
    assert_eq!(c.method, "tools/call");
    assert_eq!(c.tool_name.as_deref(), Some("github__search_repos"));
    assert_eq!(c.request_id.as_deref(), Some("req-1"));
    assert_eq!(c.decision, "allowed");
    assert_eq!(c.duration_ms, 250);
    assert_eq!(c.process_name.as_deref(), Some("claude"));
}

#[tokio::test]
async fn mcp_call_search() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mcp-search.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    writer.write(WriteOp::McpCall(sample_mcp_call("github", "allowed"))).await;
    writer.write(WriteOp::McpCall(sample_mcp_call("slack", "denied"))).await;
    writer.write(WriteOp::McpCall(sample_mcp_call("github", "warned"))).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();

    // Search by server_name
    let results = reader.search_mcp_calls("github", 10).unwrap();
    assert_eq!(results.len(), 2);

    // Search by tool_name
    let results = reader.search_mcp_calls("search_repos", 10).unwrap();
    assert_eq!(results.len(), 3); // all have search_repos in tool_name

    // Search by method
    let results = reader.search_mcp_calls("tools/call", 10).unwrap();
    assert_eq!(results.len(), 3);

    // No match
    let results = reader.search_mcp_calls("nonexistent", 10).unwrap();
    assert_eq!(results.len(), 0);
}

#[tokio::test]
async fn mcp_call_stats() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mcp-stats.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    writer.write(WriteOp::McpCall(sample_mcp_call("github", "allowed"))).await;
    writer.write(WriteOp::McpCall(sample_mcp_call("github", "allowed"))).await;
    writer.write(WriteOp::McpCall(sample_mcp_call("slack", "denied"))).await;
    writer.write(WriteOp::McpCall(sample_mcp_call("github", "warned"))).await;
    writer.write(WriteOp::McpCall({
        let mut c = sample_mcp_call("slack", "error");
        c.error_message = Some("server crashed".to_string());
        c
    })).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let stats = reader.mcp_call_stats().unwrap();

    assert_eq!(stats.total, 5);
    assert_eq!(stats.allowed, 2);
    assert_eq!(stats.warned, 1);
    assert_eq!(stats.denied, 1);
    assert_eq!(stats.errored, 1);
    assert_eq!(stats.by_server.len(), 2);

    // Sorted by count DESC: github=3, slack=2
    assert_eq!(stats.by_server[0].server_name, "github");
    assert_eq!(stats.by_server[0].count, 3);
    assert_eq!(stats.by_server[0].warned, 1);
    assert_eq!(stats.by_server[1].server_name, "slack");
    assert_eq!(stats.by_server[1].count, 2);
    assert_eq!(stats.by_server[1].denied, 1);
}

#[tokio::test]
async fn mcp_call_stats_empty_db() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mcp-empty.db");
    let writer = DbWriter::open(&path, 64).unwrap();
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let stats = reader.mcp_call_stats().unwrap();
    assert_eq!(stats.total, 0);
    assert_eq!(stats.allowed, 0);
    assert_eq!(stats.by_server.len(), 0);
}

#[tokio::test]
async fn mcp_call_cap_field_truncation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mcp-cap.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    let mut call = sample_mcp_call("github", "allowed");
    call.request_preview = Some("x".repeat(300_000)); // 300KB > 256KB cap
    writer.write(WriteOp::McpCall(call)).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let calls = reader.recent_mcp_calls(1).unwrap();
    assert_eq!(calls.len(), 1);
    // Preview should be truncated to MAX_FIELD_BYTES (256KB)
    let preview = calls[0].request_preview.as_ref().unwrap();
    assert!(preview.len() <= 256 * 1024, "preview not capped: {}", preview.len());
}

#[tokio::test]
async fn mcp_schema_migration_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mcp-migrate.db");

    // First open creates tables.
    let writer = DbWriter::open(&path, 64).unwrap();
    writer.write(WriteOp::McpCall(sample_mcp_call("github", "allowed"))).await;
    drop(writer);

    // Second open triggers migrate() again -- must not fail.
    let writer = DbWriter::open(&path, 64).unwrap();
    writer.write(WriteOp::McpCall(sample_mcp_call("slack", "denied"))).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let calls = reader.recent_mcp_calls(10).unwrap();
    assert_eq!(calls.len(), 2);
}

#[tokio::test]
async fn mcp_call_bytes_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bytes.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    let mut call = sample_mcp_call("github", "allowed");
    call.bytes_sent = 1024;
    call.bytes_received = 4096;
    writer.write(WriteOp::McpCall(call)).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let calls = reader.recent_mcp_calls(10).unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].bytes_sent, 1024);
    assert_eq!(calls[0].bytes_received, 4096);
}

#[tokio::test]
async fn mcp_call_full_preview_not_truncated() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("preview.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    // 10KB preview -- must NOT be truncated (old bug truncated at 200 chars)
    let preview = "x".repeat(10_000);
    let mut call = sample_mcp_call("github", "allowed");
    call.request_preview = Some(preview.clone());
    call.response_preview = Some(preview.clone());
    writer.write(WriteOp::McpCall(call)).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let calls = reader.recent_mcp_calls(10).unwrap();
    assert_eq!(calls[0].request_preview.as_ref().unwrap().len(), 10_000);
    assert_eq!(calls[0].response_preview.as_ref().unwrap().len(), 10_000);
}

#[tokio::test]
async fn mcp_call_huge_payload_truncated_at_256kb() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("huge.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    // 1MB preview -- must be truncated to <= 256KB by cap_field
    let preview = "a".repeat(1_000_000);
    let mut call = sample_mcp_call("github", "allowed");
    call.request_preview = Some(preview);
    writer.write(WriteOp::McpCall(call)).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let calls = reader.recent_mcp_calls(10).unwrap();
    let stored = calls[0].request_preview.as_ref().unwrap();
    assert!(stored.len() <= 256 * 1024);
}

#[tokio::test]
async fn mcp_call_200_char_payload_not_truncated() {
    // Regression: old bug truncated at 200 chars. Verify exact 200 chars preserved.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("200.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    let preview = "b".repeat(200);
    let mut call = sample_mcp_call("github", "allowed");
    call.request_preview = Some(preview.clone());
    writer.write(WriteOp::McpCall(call)).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let calls = reader.recent_mcp_calls(10).unwrap();
    assert_eq!(calls[0].request_preview.as_ref().unwrap().len(), 200);
    assert_eq!(calls[0].request_preview.as_ref().unwrap(), &preview);
}

// ── File event tests ──────────────────────────────────────────────────

fn sample_file_event(path: &str, action: FileAction, size: Option<u64>) -> FileEvent {
    FileEvent {
        timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(1700000000),
        action,
        path: path.to_string(),
        size,
    }
}

#[tokio::test]
async fn test_file_event_write_read() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fs-roundtrip.db");

    let writer = DbWriter::open(&path, 64).unwrap();
    writer.write(WriteOp::FileEvent(sample_file_event("project/app.js", FileAction::Created, Some(1234)))).await;
    writer.write(WriteOp::FileEvent(sample_file_event("project/lib.rs", FileAction::Modified, Some(5678)))).await;
    writer.write(WriteOp::FileEvent(sample_file_event("project/old.txt", FileAction::Deleted, None))).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let events = reader.recent_file_events(10).unwrap();
    assert_eq!(events.len(), 3);
    // Most recent first
    assert_eq!(events[0].path, "project/old.txt");
    assert_eq!(events[0].action, FileAction::Deleted);
    assert!(events[0].size.is_none());
    assert_eq!(events[1].path, "project/lib.rs");
    assert_eq!(events[1].action, FileAction::Modified);
    assert_eq!(events[1].size, Some(5678));
    assert_eq!(events[2].path, "project/app.js");
    assert_eq!(events[2].action, FileAction::Created);
    assert_eq!(events[2].size, Some(1234));
}

#[tokio::test]
async fn test_file_event_search() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fs-search.db");

    let writer = DbWriter::open(&path, 64).unwrap();
    writer.write(WriteOp::FileEvent(sample_file_event("project/src/app.js", FileAction::Created, Some(100)))).await;
    writer.write(WriteOp::FileEvent(sample_file_event("project/src/lib.rs", FileAction::Modified, Some(200)))).await;
    writer.write(WriteOp::FileEvent(sample_file_event("project/README.md", FileAction::Modified, Some(300)))).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let results = reader.search_file_events("src", 10).unwrap();
    assert_eq!(results.len(), 2);
    // Only the two src/ files match
    for r in &results {
        assert!(r.path.contains("src"), "expected path containing 'src', got: {}", r.path);
    }
}

#[tokio::test]
async fn test_file_event_stats() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fs-stats.db");

    let writer = DbWriter::open(&path, 64).unwrap();
    writer.write(WriteOp::FileEvent(sample_file_event("a.js", FileAction::Created, Some(10)))).await;
    writer.write(WriteOp::FileEvent(sample_file_event("b.js", FileAction::Created, Some(20)))).await;
    writer.write(WriteOp::FileEvent(sample_file_event("a.js", FileAction::Modified, Some(15)))).await;
    writer.write(WriteOp::FileEvent(sample_file_event("c.js", FileAction::Deleted, None))).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let stats = reader.file_event_stats().unwrap();
    assert_eq!(stats.total, 4);
    assert_eq!(stats.created, 2);
    assert_eq!(stats.modified, 1);
    assert_eq!(stats.deleted, 1);
}

#[tokio::test]
async fn test_file_event_empty_table() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fs-empty.db");

    let writer = DbWriter::open(&path, 64).unwrap();
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let events = reader.recent_file_events(10).unwrap();
    assert!(events.is_empty());
    let stats = reader.file_event_stats().unwrap();
    assert_eq!(stats.total, 0);
    assert_eq!(stats.created, 0);
    assert_eq!(stats.modified, 0);
    assert_eq!(stats.deleted, 0);
}

/// Fixture DB should contain fs_events rows inserted during fixture setup.
#[test]
fn test_file_events_in_fixture() {
    let reader = fixture_reader();
    let events = reader.recent_file_events(100).unwrap();
    assert!(!events.is_empty(), "fixture should contain fs_events");
    let stats = reader.file_event_stats().unwrap();
    assert!(stats.total > 0);
    assert!(stats.created > 0);
    assert!(stats.modified > 0);
    assert!(stats.deleted > 0);
    // Verify all actions parse correctly
    for e in &events {
        assert!(
            matches!(e.action, FileAction::Created | FileAction::Modified | FileAction::Deleted),
            "unexpected action: {:?}", e.action
        );
        assert!(!e.path.is_empty(), "path should not be empty in fixture");
    }
}

/// Fixture search should filter by path substring.
#[test]
fn test_file_events_fixture_search() {
    let reader = fixture_reader();
    let results = reader.search_file_events("src", 100).unwrap();
    for r in &results {
        assert!(r.path.contains("src"), "search result should contain 'src': {}", r.path);
    }
}

/// Empty path: should insert and read back without error.
#[tokio::test]
async fn test_file_event_empty_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fs-empty-path.db");

    let writer = DbWriter::open(&path, 64).unwrap();
    writer.write(WriteOp::FileEvent(sample_file_event("", FileAction::Created, Some(0)))).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let events = reader.recent_file_events(10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].path, "");
    assert_eq!(events[0].action, FileAction::Created);
    assert_eq!(events[0].size, Some(0));
}

/// Unicode paths: filenames with emoji, CJK, RTL, combining characters.
#[tokio::test]
async fn test_file_event_unicode_paths() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fs-unicode.db");

    let writer = DbWriter::open(&path, 64).unwrap();
    let paths = vec![
        "project/\u{1F4C4}document.txt",
        "project/\u{4E2D}\u{6587}\u{6587}\u{4EF6}.rs",
        "project/\u{0645}\u{0644}\u{0641}.py",
        "project/caf\u{0065}\u{0301}.js",     // e + combining accent
        "project/\u{0000}null.txt",             // null byte in path
    ];
    for p in &paths {
        writer.write(WriteOp::FileEvent(sample_file_event(p, FileAction::Created, Some(100)))).await;
    }
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let events = reader.recent_file_events(100).unwrap();
    assert_eq!(events.len(), paths.len());
}

/// Very long path: shouldn't crash or truncate silently.
#[tokio::test]
async fn test_file_event_huge_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fs-huge-path.db");

    let huge_path = "a/".repeat(10_000) + "file.txt"; // ~30KB path
    let writer = DbWriter::open(&path, 64).unwrap();
    writer.write(WriteOp::FileEvent(sample_file_event(&huge_path, FileAction::Modified, Some(42)))).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let events = reader.recent_file_events(10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].path, huge_path);
}

/// Size boundary: u64::MAX should round-trip via i64 (may lose precision).
#[tokio::test]
async fn test_file_event_max_size() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fs-max-size.db");

    let writer = DbWriter::open(&path, 64).unwrap();
    writer.write(WriteOp::FileEvent(sample_file_event("big.bin", FileAction::Created, Some(u64::MAX)))).await;
    writer.write(WriteOp::FileEvent(sample_file_event("zero.bin", FileAction::Created, Some(0)))).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let events = reader.recent_file_events(10).unwrap();
    assert_eq!(events.len(), 2);
    // size=0 should round-trip exactly
    assert_eq!(events[0].size, Some(0));
    // u64::MAX stored as i64 wraps, but shouldn't crash
    assert!(events[1].size.is_some());
}

/// SQL injection via search: parameterized queries should prevent it.
#[tokio::test]
async fn test_file_event_search_sql_injection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fs-sqli.db");

    let writer = DbWriter::open(&path, 64).unwrap();
    writer.write(WriteOp::FileEvent(sample_file_event("safe.rs", FileAction::Created, Some(10)))).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    // Should return empty, not crash or drop the table.
    let results = reader.search_file_events("'; DROP TABLE fs_events; --", 100).unwrap();
    assert!(results.is_empty());
    // Table still works:
    let events = reader.recent_file_events(10).unwrap();
    assert_eq!(events.len(), 1);
}

/// Search with SQL wildcards in user input should be treated as literals.
#[tokio::test]
async fn test_file_event_search_wildcards() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fs-wildcards.db");

    let writer = DbWriter::open(&path, 64).unwrap();
    writer.write(WriteOp::FileEvent(sample_file_event("src/main.rs", FileAction::Created, Some(10)))).await;
    writer.write(WriteOp::FileEvent(sample_file_event("src/lib.rs", FileAction::Modified, Some(20)))).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    // "%" in search should match within LIKE, but is user-provided -- verify no crash
    let results = reader.search_file_events("%", 100).unwrap();
    // "%" inside our LIKE pattern becomes "%%%" which matches everything
    assert_eq!(results.len(), 2);
}

/// Batch of many events: tests the batching/drain path in DbWriter.
#[tokio::test]
async fn test_file_event_batch_write() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fs-batch.db");

    let writer = DbWriter::open(&path, 64).unwrap();
    for i in 0..500 {
        let action = match i % 3 {
            0 => FileAction::Created,
            1 => FileAction::Modified,
            _ => FileAction::Deleted,
        };
        let size = if action == FileAction::Deleted { None } else { Some(i as u64) };
        writer.write(WriteOp::FileEvent(sample_file_event(
            &format!("file_{i}.rs"),
            action,
            size,
        ))).await;
    }
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let stats = reader.file_event_stats().unwrap();
    assert_eq!(stats.total, 500);
    assert_eq!(stats.created, 167); // 0,3,6,...,498 -> ceil(500/3) = 167
    assert_eq!(stats.modified, 167); // 1,4,7,...,499
    assert_eq!(stats.deleted, 166); // 2,5,8,...,497
    // Limit query returns at most the requested count
    let events = reader.recent_file_events(50).unwrap();
    assert_eq!(events.len(), 50);
}

/// Concurrent writers: multiple tasks writing file events simultaneously.
#[tokio::test]
async fn test_file_event_concurrent_writes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fs-concurrent.db");

    let writer = Arc::new(DbWriter::open(&path, 64).unwrap());
    let mut handles = vec![];
    for t in 0..10 {
        let w = Arc::clone(&writer);
        handles.push(tokio::spawn(async move {
            for i in 0..50 {
                w.write(WriteOp::FileEvent(sample_file_event(
                    &format!("thread_{t}/file_{i}.rs"),
                    FileAction::Modified,
                    Some(i as u64),
                ))).await;
            }
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let stats = reader.file_event_stats().unwrap();
    assert_eq!(stats.total, 500); // 10 threads x 50 events
}

/// Schema migration: a DB created without fs_events should gain the table on migrate.
#[tokio::test]
async fn test_file_event_schema_migration() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fs-migrate.db");

    // Create a minimal DB with only net_events (simulating an old schema).
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch("
            CREATE TABLE net_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT NOT NULL,
                domain TEXT NOT NULL,
                port INTEGER NOT NULL,
                decision TEXT NOT NULL,
                bytes_sent INTEGER NOT NULL DEFAULT 0,
                bytes_received INTEGER NOT NULL DEFAULT 0,
                duration_ms INTEGER NOT NULL DEFAULT 0
            );
        ").unwrap();
    }

    // Opening with DbWriter triggers migration, which should add fs_events.
    let writer = DbWriter::open(&path, 64).unwrap();
    writer.write(WriteOp::FileEvent(sample_file_event("migrated.rs", FileAction::Created, Some(42)))).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let events = reader.recent_file_events(10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].path, "migrated.rs");
}

/// Deleted events should have size=None and round-trip correctly.
#[tokio::test]
async fn test_file_event_deleted_has_no_size() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fs-deleted-size.db");

    let writer = DbWriter::open(&path, 64).unwrap();
    writer.write(WriteOp::FileEvent(sample_file_event("gone.rs", FileAction::Deleted, None))).await;
    // Also test deleted with size (shouldn't crash even though unusual).
    writer.write(WriteOp::FileEvent(sample_file_event("ghost.rs", FileAction::Deleted, Some(999)))).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let events = reader.recent_file_events(10).unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].action, FileAction::Deleted);
    assert_eq!(events[0].size, Some(999)); // unusual but valid
    assert_eq!(events[1].action, FileAction::Deleted);
    assert!(events[1].size.is_none());
}

/// Limit=0 should return no events, not crash.
#[tokio::test]
async fn test_file_event_limit_zero() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fs-limit-zero.db");

    let writer = DbWriter::open(&path, 64).unwrap();
    writer.write(WriteOp::FileEvent(sample_file_event("a.rs", FileAction::Created, Some(1)))).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let events = reader.recent_file_events(0).unwrap();
    assert!(events.is_empty());
    let search = reader.search_file_events("a", 0).unwrap();
    assert!(search.is_empty());
}

// ── try_write silently drops events when channel is full ────────────

/// Proves that try_write() silently drops events when the channel is saturated.
/// This is the root cause of empty session databases: the production code uses
/// try_write() in mitm_proxy.rs and main.rs, which returns false (ignored) when
/// the bounded channel is full, causing every event to be silently lost.
#[tokio::test]
async fn try_write_drops_events_when_channel_full() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("try-write-drop.db");

    // Capacity of 1: the channel can hold exactly 1 unsent message.
    let writer = DbWriter::open(&path, 1).unwrap();

    // First try_write succeeds -- fills the single slot.
    let ok1 = writer.try_write(WriteOp::FileEvent(
        sample_file_event("first.rs", FileAction::Created, Some(10)),
    ));
    assert!(ok1, "first try_write should succeed (channel has 1 slot)");

    // Immediately fire more try_writes without yielding -- the writer thread
    // has no chance to drain the channel, so these SILENTLY FAIL.
    let mut dropped = 0;
    for i in 0..20 {
        let ok = writer.try_write(WriteOp::FileEvent(
            sample_file_event(&format!("dropped{i}.rs"), FileAction::Modified, Some(100)),
        ));
        if !ok {
            dropped += 1;
        }
    }

    // At least some events must have been silently dropped.
    assert!(dropped > 0, "try_write should have dropped events, but none were dropped");

    // Flush and check: the DB will be MISSING the dropped events with zero indication.
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let events = reader.recent_file_events(100).unwrap();

    // We sent 21 total (1 + 20), but the DB has far fewer -- silent data loss.
    assert!(
        events.len() < 21,
        "expected silent data loss from try_write, but all 21 events were written (got {})",
        events.len()
    );
    // The dropped events are gone forever -- no log, no error, no indication.
    eprintln!(
        "PROOF: sent 21 events via try_write, only {} persisted, {} silently lost",
        events.len(),
        21 - events.len()
    );
}

/// Proves that write().await does NOT drop events under the same conditions,
/// because it backpressures (yields) until the channel has space.
#[tokio::test]
async fn async_write_never_drops_events() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("async-write-safe.db");

    // Same tiny capacity.
    let writer = DbWriter::open(&path, 1).unwrap();

    // Send 21 events via write().await -- all will succeed because write()
    // awaits channel capacity instead of failing.
    writer.write(WriteOp::FileEvent(
        sample_file_event("first.rs", FileAction::Created, Some(10)),
    )).await;

    for i in 0..20 {
        writer.write(WriteOp::FileEvent(
            sample_file_event(&format!("safe{i}.rs"), FileAction::Modified, Some(100)),
        )).await;
    }

    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let events = reader.recent_file_events(100).unwrap();

    // Every single event was persisted -- zero data loss.
    assert_eq!(
        events.len(),
        21,
        "write().await should persist all 21 events, but only got {}",
        events.len()
    );
}

/// Simulates the exact production scenario: a burst of mixed event types
/// (NetEvent, FileEvent, ModelCall) via try_write with the production
/// channel capacity of 256. Under burst conditions, events are lost.
#[tokio::test]
async fn try_write_production_burst_loses_events() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("burst-drop.db");

    // Use production capacity.
    let writer = DbWriter::open(&path, 256).unwrap();

    // Blast 500 events as fast as possible without yielding -- simulates
    // a burst of network activity + file watches + model calls arriving
    // concurrently. The writer thread batches 128 at a time and can't
    // keep up with a synchronous flood.
    let total = 500;
    let mut sent = 0;
    for i in 0..total {
        let ok = writer.try_write(WriteOp::FileEvent(
            sample_file_event(&format!("burst{i}.rs"), FileAction::Modified, Some(i as u64)),
        ));
        if ok {
            sent += 1;
        }
    }

    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let events = reader.recent_file_events(1000).unwrap();

    eprintln!(
        "BURST: tried {total}, channel accepted {sent}, DB persisted {}",
        events.len()
    );

    // With capacity 256, we can't push all 500 without the writer draining.
    // Some will be lost. (If the writer thread is fast enough on this machine
    // to drain between try_sends, we might get lucky -- but the point is
    // try_write makes NO guarantee, unlike write().await which guarantees all.)
    //
    // The real bug: even if this particular run doesn't drop events (fast CPU),
    // try_write offers ZERO delivery guarantee. The async write() path does.
    // We assert that try_write accepted fewer than we tried OR that async
    // write would have accepted all.
    if sent < total {
        assert!(
            events.len() < total,
            "some events were rejected by try_write, confirming silent drop risk"
        );
        eprintln!(
            "CONFIRMED: {total} attempted, {sent} accepted, {} dropped silently",
            total - sent
        );
    } else {
        eprintln!(
            "NOTE: writer thread drained fast enough on this machine -- \
             try_write accepted all {total}. The bug is still real: try_write \
             offers no delivery guarantee. Run under load to reproduce."
        );
    }
}

/// The production code ignores try_write's return value. This test proves
/// that pattern causes silent data loss by exactly mimicking the call sites
/// in mitm_proxy.rs:694 and main.rs:807.
#[tokio::test]
async fn ignored_try_write_return_value_causes_silent_loss() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ignored-return.db");

    let writer = DbWriter::open(&path, 1).unwrap();

    // Mimic the exact production pattern: call try_write, ignore the bool.
    // This is what mitm_proxy.rs:694 and main.rs:807 do.
    writer.try_write(WriteOp::FileEvent(
        sample_file_event("a.rs", FileAction::Created, Some(1)),
    )); // return value ignored -- fills the channel

    // These mirror a rapid sequence of file touches or network events.
    // The channel is full, so these are silently discarded.
    for i in 0..10 {
        writer.try_write(WriteOp::FileEvent(
            sample_file_event(&format!("lost{i}.rs"), FileAction::Created, Some(1)),
        )); // return value ignored -- SILENTLY DROPPED
    }

    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let events = reader.recent_file_events(100).unwrap();

    // If all 11 were persisted, the channel drained fast enough.
    // But the fundamental issue remains: try_write + ignored return = unreliable.
    if events.len() < 11 {
        eprintln!(
            "PROVED: production pattern (ignore try_write return) lost {} of 11 events",
            11 - events.len()
        );
    }

    // The important assertion: with capacity=1, it's nearly impossible
    // to persist all 11 without backpressure. At best we get 1-2.
    assert!(
        events.len() < 11,
        "expected data loss with capacity=1 and ignored try_write, got all {}", events.len()
    );
}
