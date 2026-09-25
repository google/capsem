use capsem_proto::mcp_contracts::builtin_ledger::{HttpDecision, HttpRequestRecord};

use super::*;

fn denied_request() -> HttpRequestRecord {
    HttpRequestRecord {
        timestamp_unix_ms: 1_700_000_000_000,
        domain: "localhost".to_string(),
        method: "GET".to_string(),
        path: "/".to_string(),
        decision: HttpDecision::Denied,
        status_code: None,
        bytes_sent: 0,
        bytes_received: 0,
        duration_ms: 0,
        policy_action: "block".to_string(),
        policy_rule: None,
        policy_reason: Some("non-public address".to_string()),
    }
}

#[test]
fn an_http_record_becomes_the_net_event_the_builtin_used_to_write() {
    let event = net_event(&denied_request());
    assert_eq!(event.decision, Decision::Denied);
    assert_eq!(event.domain, "localhost");
    assert_eq!(event.process_name.as_deref(), Some(BUILTIN_PROCESS_NAME));
    assert_eq!(event.conn_type.as_deref(), Some(BUILTIN_PROCESS_NAME));
    assert_eq!(event.policy_action.as_deref(), Some("block"));
    assert_eq!(event.policy_reason.as_deref(), Some("non-public address"));
    assert_eq!(event.timestamp, UNIX_EPOCH + Duration::from_millis(1_700_000_000_000));
    assert!(event.request_body.is_none() && event.response_body.is_none());
}

#[tokio::test]
async fn recording_writes_every_http_row() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let db = DbWriter::open(&db_path, 16).unwrap();
    record_builtin_ledger(&db, vec![BuiltinLedgerRecord::HttpRequest(denied_request())]).await;
    db.shutdown_blocking();

    let reader = capsem_logger::DbReader::open(&db_path).unwrap();
    let net = reader.recent_net_events(10).unwrap();
    assert_eq!(net.len(), 1);
    assert_eq!(net[0].decision, Decision::Denied);
}
