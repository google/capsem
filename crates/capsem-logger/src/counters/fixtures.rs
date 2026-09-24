//! Write ops for counter tests, built from the fields that matter to a count.
//!
//! Each builder goes through the event's own `Deserialize`, so an omitted
//! optional field takes the same default a producer's would.

use serde_json::{json, Value};

use crate::events::{
    AuditEvent, ExecEvent, ExecEventComplete, FileEvent, McpCall, ModelCall, NetEvent, SecurityAskEvent,
    SecurityRuleEvent, SubstitutionEvent,
};
use crate::writer::WriteOp;

fn from<T: serde::de::DeserializeOwned>(value: Value) -> T {
    serde_json::from_value(value).expect("fixture matches the event's shape")
}

pub(crate) fn net(decision: &str, sent: u64, received: u64) -> WriteOp {
    WriteOp::NetEvent(from::<NetEvent>(json!({
        "timestamp": 1_700_000_000.0, "domain": "example.com", "port": 443, "decision": decision,
        "bytes_sent": sent, "bytes_received": received, "duration_ms": 3,
    })))
}

pub(crate) fn model(
    provider: &str,
    model: Option<&str>,
    input: u64,
    output: u64,
    cost: f64,
    tools: &[(&str, &str)],
) -> WriteOp {
    let tool_calls: Vec<Value> = tools
        .iter()
        .enumerate()
        .map(|(index, (name, origin))| json!({"call_index": index, "call_id": format!("call-{index}"), "tool_name": name, "origin": origin}))
        .collect();
    WriteOp::ModelCall(from::<ModelCall>(json!({
        "timestamp": 1_700_000_000.0, "provider": provider, "model": model, "method": "POST",
        "path": "/v1/messages", "stream": false, "messages_count": 1, "tools_count": 0,
        "request_bytes": 10, "input_tokens": input, "output_tokens": output,
        "usage_details": {"thinking": 7}, "duration_ms": 20, "response_bytes": 30,
        "estimated_cost_usd": cost, "tool_calls": tool_calls, "tool_responses": [],
    })))
}

pub(crate) fn mcp(method: &str, server: &str, tool: &str, duration_ms: u64) -> WriteOp {
    WriteOp::McpCall(from::<McpCall>(json!({
        "timestamp": 1_700_000_000.0, "server_name": server, "method": method, "tool_name": tool,
        "decision": "allowed", "duration_ms": duration_ms, "bytes_sent": 5, "bytes_received": 9,
    })))
}

pub(crate) fn file(action: &str, path: &str) -> WriteOp {
    WriteOp::FileEvent(from::<FileEvent>(json!({
        "timestamp": 1_700_000_000.0, "action": action, "path": path, "kind": "file",
    })))
}

pub(crate) fn exec(exec_id: u64) -> WriteOp {
    WriteOp::ExecEvent(from::<ExecEvent>(json!({
        "timestamp": 1_700_000_000.0, "exec_id": exec_id, "command": "true", "source": "api",
    })))
}

pub(crate) fn exec_done(exec_id: u64) -> WriteOp {
    WriteOp::ExecEventComplete(from::<ExecEventComplete>(json!({
        "exec_id": exec_id, "exit_code": 0, "duration_ms": 4, "stdout_bytes": 0, "stderr_bytes": 0,
    })))
}

pub(crate) fn audit(exe: &str, timestamp_secs: f64) -> WriteOp {
    WriteOp::AuditEvent(from::<AuditEvent>(json!({
        "timestamp": timestamp_secs, "pid": 10, "ppid": 1, "uid": 0, "exe": exe, "argv": exe,
    })))
}

pub(crate) fn substitution(reference: &str, outcome: &str, provider: Option<&str>, timestamp_secs: f64) -> WriteOp {
    WriteOp::SubstitutionEvent(from::<SubstitutionEvent>(json!({
        "timestamp": timestamp_secs, "material_class": "credential", "source": "header",
        "algorithm": "blake3", "substitution_ref": reference, "outcome": outcome, "provider": provider,
    })))
}

pub(crate) fn rule(
    event_id: &str,
    rule_id: &str,
    action: &str,
    level: &str,
    timestamp_unix_ms: i64,
    payload: Value,
) -> WriteOp {
    WriteOp::SecurityRuleEvent(from::<SecurityRuleEvent>(json!({
        "timestamp_unix_ms": timestamp_unix_ms, "event_id": event_id, "event_type": "http.request",
        "rule_id": rule_id, "rule_action": action, "detection_level": level,
        "rule_json": "{}", "event_json": payload.to_string(),
    })))
}

pub(crate) fn ask(ask_id: &str, status: &str) -> WriteOp {
    WriteOp::SecurityAskEvent(from::<SecurityAskEvent>(json!({
        "timestamp_unix_ms": 1_700_000_000_000_i64, "ask_id": ask_id, "event_id": ask_id,
        "event_type": "http.request", "rule_id": "rule.ask", "rule_name": "ask", "status": status,
        "rule_json": "{}", "event_json": "{}",
    })))
}
