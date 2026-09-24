//! A rule row's `decision.effective` is the outcome that was enforced.
//!
//! The emitter stored the event as it arrived, whose decision defaults to
//! allow, so a blocked request, query or export was stored as allowed beside
//! `rule_action = "block"` (google/capsem#203, owned by #229). Applying the
//! rule's decision everywhere would be the opposite lie for events recorded
//! after they happened (a file the monitor saw change): nothing blocked them.
//! So a boundary that enforces the rule records the rule's decision, and a
//! record of something that already happened keeps what was enforced.
use super::*;

fn rules(action: &str, condition: &str) -> SecurityRuleSet {
    let profile = SecurityRuleProfile::parse_toml(&format!(
        r#"
[profiles.rules.guard_secret]
name = "guard_secret"
action = "{action}"
priority = 10
detection_level = "high"
match = '{condition}'
"#
    ))
    .unwrap();
    crate::net::policy_config::SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User).unwrap()
}

fn file_rules(action: &str) -> SecurityRuleSet {
    rules(action, r#"file.export.path == "/workspace/secret.txt""#)
}

fn file_rules_on(condition: &str) -> SecurityRuleSet {
    rules("block", condition)
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
async fn an_enforcing_boundary_records_the_decision_it_enforced() {
    for (action, expected) in [("block", "block"), ("allow", "allow")] {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("session.db");
        let writer = capsem_logger::DbWriter::open(&db_path, 32).unwrap();
        let emission = emit_explicit_file_security_write_and_rules_with_plugins(
            &writer,
            &file_rules(action),
            BTreeMap::new(),
            export(),
        )
        .await
        .unwrap()
        .expect("recorded");
        assert_eq!(
            emission.enforcement.action.as_str(),
            expected,
            "the caller enforces this"
        );
        writer.flush().await;
        writer.shutdown_blocking();

        let (rule_action, stored, ledger) = stored_decision(&db_path).await;
        assert_eq!(rule_action, action);
        assert_eq!(stored, expected, "rule row for an enforced `{action}` rule");
        assert_eq!(ledger, expected, "the rule row and the decision ledger agree");
    }
}

#[tokio::test]
async fn a_record_of_what_already_happened_keeps_its_outcome() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 32).unwrap();
    // The monitor saw this write after it happened; a block rule matching it
    // is a detection, not an enforcement.
    emit_file_security_write_and_rules(
        &writer,
        &file_rules_on("file.write.path == \"/workspace/secret.txt\""),
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
    .await
    .expect("file event must receive id");
    writer.flush().await;
    writer.shutdown_blocking();

    let (rule_action, stored, _) = stored_decision(&db_path).await;
    assert_eq!(
        (rule_action.as_str(), stored.as_str()),
        ("block", "allow"),
        "the rule is recorded; the write it matched was not blocked"
    );
}
