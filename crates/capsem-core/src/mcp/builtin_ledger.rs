//! Recording what the builtin MCP server did, from capsem-process.
//!
//! The builtin server describes its effects as records on its tool results
//! (see `capsem_proto::mcp_contracts::builtin_ledger`); this is where they
//! become ledger rows, written by the session's one writer.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use capsem_logger::{DbWriter, Decision, NetEvent, WriteOp};
use capsem_proto::mcp_contracts::builtin_ledger::{BuiltinLedgerRecord, HttpDecision, HttpRequestRecord};

use crate::security_engine::emit_security_write;

/// The process and connection name builtin HTTP rows carry.
pub(crate) const BUILTIN_PROCESS_NAME: &str = "mcp_builtin";

/// Milliseconds since the epoch, for stamping a record where it happened.
pub fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

fn timestamp(unix_ms: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(unix_ms)
}

/// The `net_events` row for one builtin HTTP request.
pub fn net_event(record: &HttpRequestRecord) -> NetEvent {
    NetEvent {
        event_id: None,
        timestamp: timestamp(record.timestamp_unix_ms),
        domain: record.domain.clone(),
        port: 443,
        decision: match record.decision {
            HttpDecision::Allowed => Decision::Allowed,
            HttpDecision::Denied => Decision::Denied,
        },
        process_name: Some(BUILTIN_PROCESS_NAME.to_string()),
        pid: None,
        method: Some(record.method.clone()),
        path: Some(record.path.clone()),
        query: None,
        status_code: record.status_code,
        bytes_sent: record.bytes_sent,
        bytes_received: record.bytes_received,
        duration_ms: record.duration_ms,
        matched_rule: None,
        request_headers: None,
        response_headers: None,
        request_body: None,
        response_body: None,
        conn_type: Some(BUILTIN_PROCESS_NAME.to_string()),
        policy_mode: Some("security_event".to_string()),
        policy_action: Some(record.policy_action.clone()),
        policy_rule: record.policy_rule.clone(),
        policy_reason: record.policy_reason.clone(),
        trace_id: capsem_foundation::telemetry::ambient_capsem_trace_id(),
        credential_ref: None,
    }
}

/// Write every record, in order.
pub async fn record_builtin_ledger(db: &DbWriter, records: Vec<BuiltinLedgerRecord>) {
    for record in records {
        match record {
            BuiltinLedgerRecord::HttpRequest(request) => {
                emit_security_write(db, WriteOp::NetEvent(net_event(&request))).await;
            }
        }
    }
}

#[cfg(test)]
mod tests;
