//! Net events that carry archived bodies, shaped like a model session: a
//! request and a response of word-salad text, and every fourth exchange a
//! verbatim repeat of an earlier one -- the retries and replayed tool results
//! that make up a real ledger's duplicates.

use std::time::{Duration, SystemTime};

use capsem_logger::{Decision, NetEvent, WriteOp};

/// Deterministic, poorly compressible filler: a real ledger's bodies are
/// text, not one repeated byte, and a benchmark over zeros measures nothing.
fn words(seed: usize, len: usize) -> String {
    const VOCAB: [&str; 16] = [
        "tool",
        "call",
        "result",
        "assistant",
        "content",
        "stream",
        "delta",
        "index",
        "token",
        "model",
        "input",
        "output",
        "usage",
        "message",
        "text",
        "role",
    ];
    let mut state = (seed as u64).wrapping_add(1).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    let mut out = String::with_capacity(len + 32);
    while out.len() < len {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        out.push_str(VOCAB[(state % 16) as usize]);
        out.push_str(if state.is_multiple_of(7) { "\",\"" } else { " " });
        if state.is_multiple_of(11) {
            out.push_str(&format!("{:x}", state >> 40));
        }
    }
    out.truncate(len);
    out
}

/// The `idx`-th exchange. Every fourth one repeats exchange `idx / 2`, far
/// enough back to sit in an earlier block once blocks close.
#[must_use]
pub fn net_event(idx: usize) -> WriteOp {
    let source = if idx % 4 == 3 { idx / 2 } else { idx };
    let request = format!("{{\"messages\":[\"{}\"]}}", words(source, 2048));
    let response = format!("{{\"content\":\"{}\"}}", words(source.wrapping_mul(31) + 7, 4096));
    WriteOp::NetEvent(NetEvent {
        event_id: Some(event_id(idx)),
        timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(idx as u64),
        domain: "api.bench.example".to_string(),
        port: 443,
        decision: Decision::Allowed,
        process_name: Some("bench-client".to_string()),
        pid: Some(1),
        method: Some("POST".to_string()),
        path: Some("/v1/messages".to_string()),
        query: None,
        status_code: Some(200),
        bytes_sent: request.len() as u64,
        bytes_received: response.len() as u64,
        duration_ms: 1,
        matched_rule: None,
        request_headers: Some("content-type: application/json".to_string()),
        response_headers: Some("content-type: application/json".to_string()),
        request_body: Some(request.into_bytes()),
        response_body: Some(response.into_bytes()),
        conn_type: Some("https".to_string()),
        policy_mode: None,
        policy_action: None,
        policy_rule: None,
        policy_reason: None,
        trace_id: Some(format!("{idx:016x}")),
        credential_ref: None,
    })
}

/// The id `net_event(idx)` was written under.
#[must_use]
pub fn event_id(idx: usize) -> String {
    format!("{idx:012x}")
}
