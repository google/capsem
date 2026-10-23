use super::*;
use crate::{SecurityAskEvent, SecurityAskPending, SecurityAskStatus, WriteOp};

fn pending() -> SecurityAskEvent {
    SecurityAskEvent::pending(SecurityAskPending {
        timestamp_unix_ms: 1_789_000_000_001,
        ask_id: "0123456789ac".into(),
        event_id: "0123456789ab".into(),
        event_type: "process.exec".into(),
        rule_id: "test.ask".into(),
        rule_name: "Ask".into(),
        rule_json: "{}".into(),
        event_json: r#"{"event_type":"process.exec","process":{"command":"fixture-secret"}}"#.into(),
    })
}

#[tokio::test]
async fn named_ask_query_returns_one_latest_record_and_preserves_created_time() {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(&dir.path().join("session.db")).unwrap();
    let pending = pending();
    db.write(WriteOp::SecurityAskEvent(pending.clone())).await.unwrap();
    let mut denied = pending
        .with_status(SecurityAskStatus::Denied)
        .with_reason("user_denied");
    denied.timestamp_unix_ms += 100;
    db.write(WriteOp::SecurityAskEvent(denied)).await.unwrap();
    db.flush().await.unwrap();
    let sets = execute(
        &db,
        LedgerQuery::SecurityAsks {
            limit: 10,
            ask_id: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0].rows.len(), 1);
    let wire = serde_json::to_string(&sets).unwrap();
    assert!(!wire.contains("fixture-secret"));
    let status = sets[0].columns.iter().position(|column| column == "status").unwrap();
    assert_eq!(sets[0].rows[0][status], LedgerValue::Text("denied".into()));
    let created = sets[0]
        .columns
        .iter()
        .position(|column| column == "created_at_unix_ms")
        .unwrap();
    assert_eq!(sets[0].rows[0][created], LedgerValue::Integer(1_789_000_000_001));
    let missing = execute(
        &db,
        LedgerQuery::SecurityAsks {
            limit: 1,
            ask_id: Some("aaaaaaaaaaaa".into()),
        },
    )
    .await
    .unwrap();
    assert!(missing[0].rows.is_empty());
}

#[tokio::test]
async fn resolution_without_payload_cannot_replace_original_ask_forensics() {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(&dir.path().join("session.db")).unwrap();
    let pending = pending();
    let id = pending.event_id.clone();
    db.write(WriteOp::SecurityAskEvent(pending.clone())).await.unwrap();
    db.flush().await.unwrap();
    let before = db.read_bodies(&id).await.unwrap();
    assert_eq!(before.len(), 1);
    let mut approved = pending
        .with_status(SecurityAskStatus::Approved)
        .with_resolver("gateway_user");
    approved.event_json = "{}".into();
    approved.timestamp_unix_ms += 10;
    db.write(WriteOp::SecurityAskEvent(approved)).await.unwrap();
    db.flush().await.unwrap();
    let after = db.read_bodies(&id).await.unwrap();
    assert_eq!(after.len(), before.len());
    assert_eq!(after[0].bytes, before[0].bytes);
}

#[test]
fn ask_query_rejects_unbounded_or_malformed_intent() {
    use capsem_proto::ledger::LedgerOperation;
    for query in [
        LedgerQuery::SecurityAsks { limit: 0, ask_id: None },
        LedgerQuery::SecurityAsks {
            limit: 1,
            ask_id: Some("not-an-id".into()),
        },
    ] {
        assert!(crate::ledger_protocol::LedgerCommand::Query { query }
            .validate()
            .is_err());
    }
}
