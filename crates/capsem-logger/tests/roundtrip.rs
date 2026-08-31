/// Integration tests for capsem-logger: write+read roundtrips, batching,
/// concurrent writes, shutdown, WAL concurrent access, adversarial inputs,
/// and raw SQL query endpoint.
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use capsem_logger::{
    credential_reference, validate_select_only, DbReader, DbWriter, Decision, FileAction, FileEvent, McpCall,
    ModelCall, NetEvent, ToolCallEntry, ToolResponseEntry, WriteOp,
};

/// Open the shared session fixture at tests/fixtures/session/test.db (read-only).
fn fixture_reader() -> DbReader {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop(); // crates/capsem-logger -> crates
    path.pop(); // crates -> repo root
    path.push("tests/fixtures/session/test.db");
    DbReader::open(&path).expect("failed to open fixture test.db")
}

fn sample_net_event(domain: &str, decision: Decision) -> NetEvent {
    NetEvent {
        event_id: None,
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
        request_body_full: None,
        response_body_full: None,
        conn_type: None,
        policy_mode: None,
        policy_action: None,
        policy_rule: None,
        policy_reason: None,
        trace_id: None,
        credential_ref: None,
    }
}

fn http_net_event(domain: &str) -> NetEvent {
    NetEvent {
        event_id: None,
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
        request_body_full: None,
        response_body_full: Some("{\"repos\":[]}".to_string()),
        conn_type: Some("https".to_string()),
        policy_mode: None,
        policy_action: None,
        policy_rule: None,
        policy_reason: None,
        trace_id: None,
        credential_ref: None,
    }
}

fn sample_model_call(provider: &str) -> ModelCall {
    ModelCall {
        event_id: None,
        timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(1700000000),
        provider: provider.to_string(),
        protocol: Some(provider.to_string()),
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
        request_body_full: Some("{\"model\":\"...\"}".to_string()),
        message_id: Some("msg_01".to_string()),
        status_code: Some(200),
        text_content: Some("Hello world!".to_string()),
        thinking_content: None,
        response_body_full: Some("{\"content\":[{\"text\":\"Hello world!\"}]}".to_string()),
        stop_reason: Some("end_turn".to_string()),
        input_tokens: Some(25),
        output_tokens: Some(10),
        usage_details: std::collections::BTreeMap::new(),
        duration_ms: 1500,
        response_bytes: 4096,
        estimated_cost_usd: 0.001,
        trace_id: None,
        credential_ref: None,
        tool_calls: vec![ToolCallEntry {
            call_index: 0,
            call_id: "toolu_01".to_string(),
            tool_name: "get_weather".to_string(),
            arguments: Some("{\"city\":\"NYC\"}".to_string()),
            origin: "native".to_string(),
            trace_id: None,
        }],
        tool_responses: vec![ToolResponseEntry {
            call_id: "toolu_prev".to_string(),
            content_preview: Some("72F and sunny".to_string()),
            is_error: false,
            trace_id: None,
            credential_ref: None,
        }],
    }
}

// ── File-backed write+read roundtrips ────────────────────────────────

#[path = "roundtrip/analytics.rs"]
mod analytics;
#[path = "roundtrip/event_roundtrips.rs"]
mod event_roundtrips;
#[path = "roundtrip/file_events.rs"]
mod file_events;
#[path = "roundtrip/mcp_calls.rs"]
mod mcp_calls;
#[path = "roundtrip/reader_queries.rs"]
mod reader_queries;
#[path = "roundtrip/tool_dedup.rs"]
mod tool_dedup;

fn sample_mcp_call(server: &str, decision: &str) -> McpCall {
    McpCall {
        event_id: None,
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
        transport: "vsock_frame".to_string(),
        policy_mode: Some("audit_only".to_string()),
        policy_action: Some(decision_to_policy_action(decision).to_string()),
        policy_rule: Some(format!("mcp.tool.{server}__search_repos")),
        policy_reason: Some(format!("local policy {decision}")),
        trace_id: None,
        credential_ref: None,
    }
}

fn mcp_tool_rows(reader: &DbReader) -> Vec<BTreeMap<String, serde_json::Value>> {
    let json = reader
        .query_raw(
            "SELECT event_id, timestamp, server_name, method, tool_name, request_id,
                    arguments AS request_preview, response_preview, decision, duration_ms,
                    error_message, process_name, bytes_sent, bytes_received, policy_mode,
                    policy_action, policy_rule, policy_reason, origin, transport
             FROM tool_calls
             WHERE origin = 'mcp'
             ORDER BY id DESC",
        )
        .unwrap();
    let (_, rows) = parse_query_result(&json);
    rows
}

fn decision_to_policy_action(decision: &str) -> &'static str {
    match decision {
        "denied" => "deny",
        _ => "allow",
    }
}

fn parse_query_result(json: &str) -> (Vec<String>, Vec<BTreeMap<String, serde_json::Value>>) {
    let value: serde_json::Value = serde_json::from_str(json).unwrap();
    let columns: Vec<String> = value["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|column| column.as_str().unwrap().to_string())
        .collect();
    let rows = value["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            columns
                .iter()
                .zip(row.as_array().unwrap())
                .map(|(column, value)| (column.clone(), value.clone()))
                .collect()
        })
        .collect();
    (columns, rows)
}
