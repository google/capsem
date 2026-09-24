//! A rule row's `event_json.decision.effective` is the outcome that was
//! enforced, not the event as it arrived.
//!
//! The emitter applied the selected rule to a separate decision state and then
//! serialized the incoming event, whose decision defaults to allow. A blocked
//! HTTP request, DNS query, file operation or process was therefore stored with
//! `decision.effective = "allow"` beside `rule_action = "block"`
//! (google/capsem#203, owned by #229).
use super::*;

fn file_rules(action: &str) -> SecurityRuleSet {
    let profile = SecurityRuleProfile::parse_toml(&format!(
        r#"
[profiles.rules.guard_secret]
name = "guard_secret"
action = "{action}"
priority = 10
detection_level = "high"
match = 'file.export.path == "/workspace/secret.txt"'
"#
    ))
    .unwrap();
    crate::net::policy_config::SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User).unwrap()
}

fn export() -> ExplicitFileSecurityEvent {
    ExplicitFileSecurityEvent {
        action: FileAction::Exported,
        path: "/workspace/secret.txt".to_string(),
        size: Some(4),
        content: Some("body".to_string()),
        mime_type: Some("text/plain".to_string()),
        trace_id: None,
        credential_ref: None,
    }
}

/// `(rule_action, the rule row's decision.effective, the decision ledger's effective)`.
async fn stored_decision(db_path: &std::path::Path) -> (String, String, String) {
    let (event_id, rule_action, effective) = {
        let conn = rusqlite::Connection::open(db_path).unwrap();
        let (event_id, rule_action): (String, String) = conn
            .query_row("SELECT event_id, rule_action FROM security_rule_events", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        let effective: String = conn
            .query_row(
                "SELECT COALESCE(d.effective_decision, r.effective_decision) \
                 FROM security_decision_events d LEFT JOIN security_decision_runs r ON r.id = d.run_id",
                [],
                |row| row.get(0),
            )
            .unwrap();
        (event_id, rule_action, effective)
    };
    let payload: serde_json::Value = serde_json::from_str(
        &super::decision_ledger::archived_payload_of(db_path, "security_rule_events", &event_id).await,
    )
    .unwrap();
    let stored = payload["decision"]["effective"]
        .as_str()
        .unwrap_or_else(|| panic!("rule payload carries decision.effective: {payload}"))
        .to_string();
    (rule_action, stored, effective)
}

#[tokio::test]
async fn a_blocking_rule_row_stores_the_block_it_enforced() {
    for (action, expected) in [("block", "block"), ("allow", "allow")] {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("session.db");
        let writer = capsem_logger::DbWriter::open(&db_path, 32).unwrap();
        emit_explicit_file_security_write_and_rules(&writer, &file_rules(action), export())
            .await
            .expect("explicit file event must receive id");
        writer.flush().await;
        writer.shutdown_blocking();

        let (rule_action, stored, effective) = stored_decision(&db_path).await;
        assert_eq!(rule_action, action);
        assert_eq!(stored, expected, "rule row for an `{action}` rule");
        assert_eq!(effective, expected, "the rule row and the decision ledger agree");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_blocking_emitter_stores_the_block_it_enforced() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 32).unwrap();
    let profile = SecurityRuleProfile::parse_toml(
        r#"
[profiles.rules.guard_write]
name = "guard_write"
action = "block"
priority = 10
detection_level = "high"
match = 'file.write.path == "/workspace/secret.txt"'
"#,
    )
    .unwrap();
    let rules =
        crate::net::policy_config::SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User).unwrap();
    emit_file_security_write_and_rules_blocking(
        &writer,
        &rules,
        FileEvent {
            event_id: None,
            timestamp: SystemTime::now(),
            action: FileAction::Modified,
            path: "/workspace/secret.txt".to_string(),
            size: Some(4),
            kind: capsem_logger::FileKind::File,
            trace_id: None,
            credential_ref: None,
        },
    )
    .expect("file event must receive id");
    writer.shutdown_blocking();

    let (rule_action, stored, _) = stored_decision(&db_path).await;
    assert_eq!((rule_action.as_str(), stored.as_str()), ("block", "block"));
}
