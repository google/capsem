use super::*;

fn emission_with(
    action: capsem_core::security_engine::SecurityEnforcementAction,
    rule_id: Option<&str>,
    reason: Option<&str>,
) -> capsem_core::security_engine::SecurityRuleEmission {
    capsem_core::security_engine::SecurityRuleEmission {
        event_id: capsem_core::security_engine::SecurityEventId::parse("0123456789ab").unwrap(),
        emitted: 1,
        enforcement: capsem_core::security_engine::SecurityEnforcementDecision {
            action,
            rule_id: rule_id.map(str::to_string),
            rule_name: rule_id.map(str::to_string),
            reason: reason.map(str::to_string),
            ask_id: None,
        },
        event: capsem_core::security_engine::SecurityEvent::new(
            capsem_core::security_engine::RuntimeSecurityEventType::ProcessExec,
        ),
        rule_events: Vec::new(),
    }
}

#[test]
fn exec_boundary_allows_only_an_allow_decision() {
    use capsem_core::security_engine::SecurityEnforcementAction as Action;

    assert_eq!(
        exec_boundary_refusal(1, &Ok(Some(emission_with(Action::Allow, None, None)))),
        None,
        "an allowing boundary must dispatch the command"
    );

    let blocked = exec_boundary_refusal(
        2,
        &Ok(Some(emission_with(
            Action::Block,
            Some("profiles.rules.guard_curl"),
            Some("curl is not allowed"),
        ))),
    );
    assert_eq!(
        blocked.as_deref(),
        Some("curl is not allowed"),
        "the rule's own reason is what the caller sees"
    );

    let asked = exec_boundary_refusal(
        3,
        &Ok(Some(emission_with(
            Action::Ask,
            Some("profiles.rules.guard_curl"),
            None,
        ))),
    );
    assert_eq!(
        asked.as_deref(),
        Some("capsem: command requires approval by security rule: profiles.rules.guard_curl"),
        "an ask with no resolution path still withholds the command"
    );
}

#[test]
fn exec_boundary_refuses_when_it_cannot_decide() {
    let unwritten = exec_boundary_refusal(4, &Ok(None));
    assert_eq!(
        unwritten.as_deref(),
        Some("capsem: command refused, security ledger unavailable"),
        "a boundary that could not be recorded must not dispatch"
    );

    let failed = exec_boundary_refusal(5, &Err("rule set is broken".to_string()));
    assert_eq!(
        failed.as_deref(),
        Some("capsem: command refused, security evaluation failed: rule set is broken"),
        "an unevaluated boundary must not dispatch"
    );
}

// -----------------------------------------------------------------------
// Audit port: rules are read per record, not snapshotted per connection
// -----------------------------------------------------------------------

fn audit_rules(rule_key: &str, exe: &str) -> capsem_core::net::policy_config::SecurityRuleSet {
    let profile = capsem_core::net::policy_config::SecurityRuleProfile::parse_toml(&format!(
        r#"
[profiles.rules.{rule_key}]
name = "{rule_key}"
action = "allow"
detection_level = "informational"
match = 'process.exec.path == "{exe}"'
"#
    ))
    .expect("rules parse");
    capsem_core::net::policy_config::SecurityRuleSet::compile_profile(
        &profile,
        capsem_core::net::policy_config::SecurityRuleSource::User,
    )
    .expect("rules compile")
}

fn audit_payload(exe: &str) -> Vec<u8> {
    let frame = capsem_proto::encode_audit_record(&capsem_proto::AuditRecord {
        timestamp_us: 1_700_000_000_000_000,
        pid: 4242,
        ppid: 1,
        uid: 0,
        exe: exe.to_string(),
        comm: Some("cmd".to_string()),
        argv: exe.to_string(),
        cwd: Some("/root".to_string()),
        tty: None,
        session_id: None,
        parent_exe: Some("/bin/bash".to_string()),
        audit_id: format!("audit-{}", exe.trim_start_matches('/').replace('/', "-")),
    })
    .expect("audit record encodes");
    frame[4..].to_vec()
}

fn matched_rule_ids(db_path: &std::path::Path) -> Vec<String> {
    let reader = capsem_logger::DbReader::open(db_path).unwrap();
    let rows: serde_json::Value = serde_json::from_str(
        &reader
            .query_raw("SELECT rule_id FROM security_rule_events ORDER BY rule_id")
            .expect("rule events readable"),
    )
    .unwrap();
    rows["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row[0].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn audit_records_are_evaluated_against_the_rules_current_at_arrival() {
    use std::sync::{Arc, RwLock};

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let db = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    let handle = RwLock::new(Arc::new(audit_rules("boot_rules", "/bin/first")));

    handle_audit_frame(&audit_payload("/bin/first"), &db, &handle);
    // A profile edit reloads the rules while the audit connection stays up.
    *handle.write().unwrap() = Arc::new(audit_rules("reloaded_rules", "/bin/second"));
    handle_audit_frame(&audit_payload("/bin/second"), &db, &handle);
    db.shutdown_blocking();

    assert_eq!(
        matched_rule_ids(&db_path),
        vec![
            "profiles.rules.boot_rules".to_string(),
            "profiles.rules.reloaded_rules".to_string()
        ],
        "the second record must be judged by the reloaded rules"
    );
}

#[test]
fn serve_audit_records_handles_every_frame_on_the_stream() {
    use std::sync::{Arc, RwLock};

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let db = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    let handle = RwLock::new(Arc::new(audit_rules("seen", "/bin/first")));

    let mut stream = Vec::new();
    for exe in ["/bin/first", "/bin/other", "/bin/first"] {
        let payload = audit_payload(exe);
        stream.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        stream.extend_from_slice(&payload);
    }
    serve_audit_records(&mut std::io::Cursor::new(stream), &db, &handle);
    db.shutdown_blocking();

    assert_eq!(
        matched_rule_ids(&db_path),
        vec!["profiles.rules.seen".to_string(), "profiles.rules.seen".to_string()]
    );
}
