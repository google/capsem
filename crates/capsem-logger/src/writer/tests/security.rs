//! What the writer stores for a security rule match, ask and decision.
//!
//! The row is the queryable half: rule id, action, detection level, the
//! columns every projection filters on. A rule match's forensic payload is the
//! other half and lives in the session archive, so the roundtrip is preserved
//! only if both come back.

use super::*;

#[tokio::test]
async fn security_rule_event_roundtrip_preserves_forensic_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("security-rule.db");
    let writer = DbWriter::open(&db_path, 64).unwrap();

    writer
        .write(WriteOp::SecurityRuleEvent(crate::events::SecurityRuleEvent {
            timestamp_unix_ms: 1_789_000_000_000,
            event_id: "abcdef123456".into(),
            event_type: "model.call".into(),
            rule_id: "openai_api_block".into(),
            rule_action: crate::events::SecurityRuleAction::Block,
            detection_level: crate::events::SecurityDetectionLevel::Critical,
            rule_json: r#"{"name":"openai_api_block","match":"model.provider == \"openai\""}"#.into(),
            event_json: r#"{"common":{"event_type":"model.call"},"model":{"provider":"openai"}}"#.into(),
            trace_id: Some("trace_abc".into()),
            turn_id: Some("turn_abc".into()),
            credential_ref: Some(crate::events::credential_reference("openai", "sk-test")),
        }))
        .await;
    writer.flush().await;
    drop(writer);

    let reader = crate::reader::DbReader::open(&db_path).unwrap();
    let events = reader.recent_security_rule_events(10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_id, "abcdef123456");
    assert_eq!(events[0].event_type, "model.call");
    assert_eq!(events[0].rule_id, "openai_api_block");
    assert_eq!(events[0].rule_action, crate::events::SecurityRuleAction::Block);
    assert_eq!(
        events[0].detection_level,
        crate::events::SecurityDetectionLevel::Critical
    );
    assert!(events[0].rule_json.contains("openai_api_block"));
    assert_eq!(events[0].trace_id.as_deref(), Some("trace_abc"));
    assert_eq!(events[0].turn_id.as_deref(), Some("turn_abc"));
    assert!(events[0]
        .credential_ref
        .as_deref()
        .is_some_and(crate::events::is_credential_reference));

    // The forensic payload is not in the row any more; it is this event's one
    // body, and the roundtrip holds only if it reads back whole.
    let payload = crate::DbHandle::open_external_reader(&db_path)
        .unwrap()
        .read_body("abcdef123456", "security_rule_events", crate::BodyDirection::Payload)
        .await
        .unwrap()
        .expect("the matched event payload is archived");
    assert_eq!(payload.content_type.as_deref(), Some("application/json"));
    assert_eq!(
        payload.bytes,
        br#"{"common":{"event_type":"model.call"},"model":{"provider":"openai"}}"#
    );
}

#[tokio::test]
async fn security_ask_event_roundtrip_preserves_lifecycle_rows() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("security-ask.db");
    let writer = DbWriter::open(&db_path, 64).unwrap();
    let pending = crate::events::SecurityAskEvent::pending(crate::events::SecurityAskPending {
        timestamp_unix_ms: 1_789_000_000_000,
        ask_id: "abcdef123456".to_string(),
        event_id: "111111abcdef".to_string(),
        event_type: "http.request".to_string(),
        rule_id: "profiles.rules.ask_openai".to_string(),
        rule_name: "ask_openai".to_string(),
        rule_json: r#"{"name":"ask_openai"}"#.to_string(),
        event_json: r#"{"http":{"host":"api.openai.com"}}"#.to_string(),
    })
    .with_trace_id("trace_ask");
    let approved = pending
        .clone()
        .with_status(crate::events::SecurityAskStatus::Approved)
        .with_resolver("tester")
        .with_reason("approved");

    writer.write(WriteOp::SecurityAskEvent(pending.clone())).await;
    writer.write(WriteOp::SecurityAskEvent(approved)).await;
    drop(writer);

    let reader = crate::reader::DbReader::open(&db_path).unwrap();
    let rows = reader.recent_security_ask_events(10).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].status, crate::events::SecurityAskStatus::Approved);
    assert_eq!(rows[0].resolver.as_deref(), Some("tester"));
    assert_eq!(rows[1].status, crate::events::SecurityAskStatus::Pending);
    assert_eq!(rows[1].event_id, "111111abcdef");
    assert_eq!(rows[1].rule_id, "profiles.rules.ask_openai");
    let latest = reader.latest_security_ask_event("abcdef123456").unwrap().unwrap();
    assert_eq!(latest.status, crate::events::SecurityAskStatus::Approved);

    // The asked-about event is archived beside the lifecycle rows, once: the
    // pending row and its resolution name the same event and carry the same
    // bytes, and the index holds one body per (event, table, direction).
    let db = crate::DbHandle::open_external_reader(&db_path).unwrap();
    let payload = db
        .read_body("111111abcdef", "security_ask_events", crate::BodyDirection::Payload)
        .await
        .unwrap()
        .expect("the asked-about event is archived");
    assert_eq!(payload.bytes, br#"{"http":{"host":"api.openai.com"}}"#);
    assert_eq!(payload.content_type.as_deref(), Some("application/json"));
    let bodies = db.read_bodies("111111abcdef").await.unwrap();
    assert_eq!(bodies.len(), 1, "two lifecycle rows of one ask index one body, not two");
}

#[tokio::test]
async fn security_decision_event_roundtrip_preserves_explicit_transition() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("security-decision.db");
    let writer = DbWriter::open(&db_path, 64).unwrap();

    writer
        .write(WriteOp::SecurityDecisionEvent(crate::events::SecurityDecisionEvent {
            timestamp_unix_ms: 1_789_000_000_000,
            event_id: "abcdef123456".into(),
            event_type: "file.import".into(),
            stage: crate::events::SecurityDecisionStage::Rewrite,
            actor: "dummy_pre_eicar".into(),
            rule_id: Some("profiles.rules.scan_eicar".into()),
            plugin_id: Some("dummy_pre_eicar".into()),
            previous_decision: crate::events::SecurityDecision::Allow,
            requested_decision: crate::events::SecurityDecision::Block,
            effective_decision: crate::events::SecurityDecision::Block,
            reason: Some("EICAR test seed observed".into()),
            event_json: r#"{"file":{"import":{"name":"eicar.txt"}}}"#.into(),
            trace_id: Some("trace_eicar".into()),
            turn_id: Some("turn_eicar".into()),
            credential_ref: Some(crate::events::credential_reference("github", "ghp-test")),
        }))
        .await;
    drop(writer);

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let row: (String, String, String, String, String, String, String, String, String) = conn
        .query_row(
            "SELECT stage, actor, previous_decision, requested_decision,
                    effective_decision, reason, trace_id, turn_id, credential_ref
             FROM security_decision_events WHERE event_id = 'abcdef123456'",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        row,
        (
            "rewrite".into(),
            "dummy_pre_eicar".into(),
            "allow".into(),
            "block".into(),
            "block".into(),
            "EICAR test seed observed".into(),
            "trace_eicar".into(),
            "turn_eicar".into(),
            crate::events::credential_reference("github", "ghp-test"),
        )
    );
    drop(conn);

    // The transition's row keeps what projections filter on; the event it was
    // made about is archived, and the roundtrip holds only if it reads back.
    let payload = crate::DbHandle::open_external_reader(&db_path)
        .unwrap()
        .read_body(
            "abcdef123456",
            "security_decision_events",
            crate::BodyDirection::Payload,
        )
        .await
        .unwrap()
        .expect("the decided-about event is archived");
    assert_eq!(payload.bytes, br#"{"file":{"import":{"name":"eicar.txt"}}}"#);
    assert_eq!(payload.source_table, "security_decision_events");
}

#[tokio::test]
async fn security_rule_stats_are_regenerated_from_session_db() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("security-rule-stats.db");
    let writer = DbWriter::open(&db_path, 64).unwrap();

    for (idx, action, level) in [
        (
            1,
            crate::events::SecurityRuleAction::Block,
            crate::events::SecurityDetectionLevel::Critical,
        ),
        (
            2,
            crate::events::SecurityRuleAction::Block,
            crate::events::SecurityDetectionLevel::Critical,
        ),
        (
            3,
            crate::events::SecurityRuleAction::Allow,
            crate::events::SecurityDetectionLevel::None,
        ),
    ] {
        writer
            .write(WriteOp::SecurityRuleEvent(crate::events::SecurityRuleEvent {
                timestamp_unix_ms: 1_789_000_000_000 + idx,
                event_id: format!("{idx:012x}"),
                event_type: if idx == 3 {
                    "http.request".into()
                } else {
                    "model.call".into()
                },
                rule_id: if idx == 3 {
                    "github_api_allow".into()
                } else {
                    "openai_api_block".into()
                },
                rule_action: action,
                detection_level: level,
                rule_json: "{}".into(),
                event_json: "{}".into(),
                trace_id: None,
                turn_id: None,
                credential_ref: None,
            }))
            .await;
    }
    drop(writer);

    let reader = crate::reader::DbReader::open(&db_path).unwrap();
    let stats = reader.security_rule_stats().unwrap();
    assert_eq!(stats.total, 3);
    assert!(stats
        .by_action
        .iter()
        .any(|entry| entry.rule_action == "block" && entry.count == 2));
    assert!(stats
        .by_event_type
        .iter()
        .any(|entry| entry.event_type == "model.call" && entry.count == 2));
    let block = stats
        .by_rule
        .iter()
        .find(|entry| entry.rule_id == "openai_api_block")
        .unwrap();
    assert_eq!(block.rule_action, "block");
    assert_eq!(block.detection_level, "critical");
    assert_eq!(block.count, 2);
    assert_eq!(block.latest_event_id, "000000000002");
}
