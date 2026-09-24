use capsem_proto::mcp_contracts::builtin_ledger::{FileRevertedRecord, HttpDecision, HttpRequestRecord, RevertAction};

use super::*;
use crate::net::policy_config::{SecurityRuleProfile, SecurityRuleSource};

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

fn restored_file() -> FileRevertedRecord {
    FileRevertedRecord {
        timestamp_unix_ms: 1_700_000_000_001,
        path: "important.txt".to_string(),
        checkpoint: "cp-0".to_string(),
        action: RevertAction::Restored,
        size: Some(8),
    }
}

/// A rule that matches every file event, so a file record owes a rule row.
fn every_file_rule() -> SecurityRuleSet {
    let profile = SecurityRuleProfile::parse_toml(
        r#"
        [profiles.rules.every_file]
        name = "every_file"
        action = "allow"
        detection_level = "informational"
        match = 'file.kind == "file"'
        "#,
    )
    .unwrap();
    SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User).unwrap()
}

fn count(reader: &capsem_logger::DbReader, table: &str) -> i64 {
    let rows: serde_json::Value =
        serde_json::from_str(&reader.query_raw(&format!("SELECT COUNT(*) FROM {table}")).unwrap()).unwrap();
    rows["rows"][0][0].as_i64().unwrap()
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

#[test]
fn a_revert_record_names_its_checkpoint_in_the_path() {
    let event = file_event(&restored_file());
    assert_eq!(event.action, FileAction::Restored);
    assert_eq!(event.path, "important.txt (from cp-0)");
    assert_eq!(event.size, Some(8));

    let deleted = file_event(&FileRevertedRecord {
        action: RevertAction::Deleted,
        size: None,
        ..restored_file()
    });
    assert_eq!(deleted.action, FileAction::Deleted);
}

/// capsem-process writes the rows, and judges the file record against the
/// rule set it is handed -- the live one -- not the builtin's startup copy.
#[tokio::test]
async fn recording_writes_every_row_with_the_rules_it_is_given() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let db = DbWriter::open(&db_path, 16).unwrap();
    record_builtin_ledger(
        &db,
        &every_file_rule(),
        vec![
            BuiltinLedgerRecord::HttpRequest(denied_request()),
            BuiltinLedgerRecord::FileReverted(restored_file()),
        ],
    )
    .await;
    db.shutdown_blocking();

    let reader = capsem_logger::DbReader::open(&db_path).unwrap();
    let net = reader.recent_net_events(10).unwrap();
    assert_eq!(net.len(), 1);
    assert_eq!(net[0].decision, Decision::Denied);
    let files = reader.recent_file_events(10).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "important.txt (from cp-0)");
    assert_eq!(
        count(&reader, "security_rule_events"),
        1,
        "the file record is judged by the rules capsem-process holds"
    );
}
