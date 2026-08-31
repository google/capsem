use super::*;

/// The rule action a boundary fixture compiles, paired with the plugin policy
/// applied over it. Both stages feed the same escalate-only decision state, so
/// the matrix below is the contract for how the two combine.
fn boundary_enforcement_for(
    rule_action: &str,
    plugin_id: &str,
    mode: SecurityPluginMode,
    content: &str,
) -> SecurityEnforcementAction {
    let profile = SecurityRuleProfile::parse_toml(&format!(
        r#"
[default.file]
name = "file"
action = "{rule_action}"
priority = "default"
match = "has(file.export.path)"
"#
    ))
    .expect("profile parses");
    let rules = SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::BuiltinDefault).expect("rules compile");
    let mut plugin_policy = BTreeMap::new();
    plugin_policy.insert(
        plugin_id.to_string(),
        SecurityPluginConfig {
            mode,
            detection_level: crate::net::policy_config::DetectionLevel::High,
        },
    );
    let event = SecurityEvent::new(RuntimeSecurityEventType::FileExport).with_file(FileSecurityEvent {
        export_path: Some("/workspace/x.txt".to_string()),
        export_content: Some(content.to_string()),
        ..Default::default()
    });

    evaluate_security_boundary(&rules, plugin_policy, event)
        .expect("boundary evaluates")
        .enforcement
        .action
}

#[test]
fn plugin_modes_escalate_the_boundary_decision_at_both_stages() {
    use SecurityEnforcementAction::{Allow, Ask, Block};
    use SecurityPluginMode as Mode;

    // (rule action, plugin, mode, content, expected enforcement)
    let matrix = [
        // A postprocess plugin used to be read for `block` only, so `ask` was
        // silently downgraded to `allow`.
        ("allow", "dummy_post_allow", Mode::Ask, "plain", Ask),
        ("allow", "dummy_post_allow", Mode::Block, "plain", Block),
        ("allow", "dummy_post_allow", Mode::Allow, "plain", Allow),
        ("allow", "dummy_post_allow", Mode::Rewrite, "plain", Allow),
        // A preprocess plugin has always been read for both.
        ("allow", "dummy_pre_eicar", Mode::Ask, "EICAR", Ask),
        ("allow", "dummy_pre_eicar", Mode::Block, "EICAR", Block),
        ("allow", "dummy_pre_eicar", Mode::Allow, "EICAR", Allow),
        // A plugin that does not apply leaves the rule verdict alone.
        ("allow", "dummy_pre_eicar", Mode::Block, "plain", Allow),
        ("allow", "dummy_pre_eicar", Mode::Ask, "plain", Allow),
        // Escalate-only: no plugin mode can talk a stricter rule down.
        ("block", "dummy_post_allow", Mode::Allow, "plain", Block),
        ("block", "dummy_post_allow", Mode::Ask, "plain", Block),
        ("block", "dummy_pre_eicar", Mode::Allow, "EICAR", Block),
        ("ask", "dummy_post_allow", Mode::Allow, "plain", Ask),
        ("ask", "dummy_post_allow", Mode::Block, "plain", Block),
        // A disabled plugin never runs, whatever the stage.
        ("allow", "dummy_post_allow", Mode::Disable, "plain", Allow),
        ("allow", "dummy_pre_eicar", Mode::Disable, "EICAR", Allow),
    ];

    for (rule_action, plugin_id, mode, content, expected) in matrix {
        let actual = boundary_enforcement_for(rule_action, plugin_id, mode, content);
        assert_eq!(
            actual,
            expected,
            "rule '{rule_action}' with plugin '{plugin_id}' in '{}' mode over {content:?} \
             must enforce {expected:?}, got {actual:?}",
            mode.as_str()
        );
    }
}

#[test]
fn postprocess_plugin_ask_is_recorded_on_the_event_and_the_decision() {
    let profile = SecurityRuleProfile::parse_toml(
        r#"
[default.file]
name = "file"
action = "allow"
priority = "default"
match = "has(file.export.path)"
"#,
    )
    .expect("profile parses");
    let rules = SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::BuiltinDefault).expect("rules compile");
    let mut plugin_policy = BTreeMap::new();
    plugin_policy.insert(
        "dummy_post_allow".to_string(),
        SecurityPluginConfig {
            mode: SecurityPluginMode::Ask,
            detection_level: crate::net::policy_config::DetectionLevel::High,
        },
    );
    let event = SecurityEvent::new(RuntimeSecurityEventType::FileExport).with_file(FileSecurityEvent {
        export_path: Some("/workspace/x.txt".to_string()),
        ..Default::default()
    });

    let evaluation = evaluate_security_boundary(&rules, plugin_policy, event).expect("boundary evaluates");

    assert_eq!(
        evaluation.enforcement.action,
        SecurityEnforcementAction::Ask,
        "enforcement must agree with the decision state the plugin produced"
    );
    assert_eq!(evaluation.event.decision.effective, SecurityDecisionKind::Ask);
    let detection = evaluation
        .event
        .detections
        .iter()
        .find(|detection| detection.plugin_id.as_deref() == Some("dummy_post_allow"))
        .expect("the asking plugin records a detection");
    assert_eq!(detection.plugin_mode, Some(SecurityPluginMode::Ask));
    assert_eq!(detection.source, SecurityDetectionSource::Plugin);
}

fn exec_boundary_rules(action: &str) -> SecurityRuleSet {
    let profile = SecurityRuleProfile::parse_toml(&format!(
        r#"
[profiles.rules.guard_curl]
name = "guard_curl"
action = "{action}"
priority = 10
detection_level = "high"
reason = "curl is not allowed in this profile"
match = 'process.command.contains("curl")'

[default.process]
name = "process"
action = "allow"
priority = "default"
match = "has(process.exec.path) || has(process.command) || has(process.exec.id)"
"#
    ))
    .expect("profile parses");
    SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User).expect("rules compile")
}

fn exec_event(exec_id: u64, command: &str) -> ExecEvent {
    ExecEvent {
        event_id: None,
        timestamp: SystemTime::now(),
        exec_id,
        command: command.to_string(),
        source: "api".to_string(),
        trace_id: Some("trace_exec".to_string()),
        process_name: None,
        credential_ref: None,
    }
}

#[tokio::test]
async fn process_exec_boundary_blocks_before_the_command_is_dispatched() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 32).unwrap();
    let rules = exec_boundary_rules("block");

    let emission = emit_process_exec_security_boundary(
        &writer,
        &rules,
        BTreeMap::new(),
        exec_event(7, "curl https://evil.test"),
    )
    .await
    .expect("boundary evaluates")
    .expect("boundary writes a primary row");

    assert_eq!(
        emission.enforcement.action,
        SecurityEnforcementAction::Block,
        "a blocking process rule must produce a blocking decision, not just a ledger row"
    );
    assert_eq!(
        emission.enforcement.rule_id.as_deref(),
        Some("profiles.rules.guard_curl")
    );
    assert_eq!(
        emission.enforcement.reason.as_deref(),
        Some("curl is not allowed in this profile"),
        "the caller needs the rule's reason to tell the user why"
    );

    writer.flush().await;
    writer.shutdown_blocking();
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let exec_rows: i64 = conn
        .query_row("SELECT count(*) FROM exec_events", [], |row| row.get(0))
        .unwrap();
    let blocked: i64 = conn
        .query_row(
            "SELECT count(*) FROM security_rule_events WHERE rule_action = 'block'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(exec_rows, 1, "the attempt is still on the record");
    assert_eq!(blocked, 1, "and so is the rule that refused it");
}

#[tokio::test]
async fn process_exec_boundary_allows_an_unmatched_command_and_carries_its_event_id() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 32).unwrap();
    let rules = exec_boundary_rules("block");

    let emission = emit_process_exec_security_boundary(&writer, &rules, BTreeMap::new(), exec_event(8, "echo hello"))
        .await
        .expect("boundary evaluates")
        .expect("boundary writes a primary row");

    assert_eq!(emission.enforcement.action, SecurityEnforcementAction::Allow);

    writer.flush().await;
    writer.shutdown_blocking();
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let logged_id: String = conn
        .query_row("SELECT event_id FROM exec_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        logged_id,
        emission.event_id.as_str(),
        "the emission must name the exec row so the completion can correlate to it"
    );
}

#[tokio::test]
async fn process_exec_boundary_asks_when_the_rule_asks() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 32).unwrap();
    let rules = exec_boundary_rules("ask");

    let emission = emit_process_exec_security_boundary(
        &writer,
        &rules,
        BTreeMap::new(),
        exec_event(9, "curl https://evil.test"),
    )
    .await
    .expect("boundary evaluates")
    .expect("boundary writes a primary row");

    assert_eq!(emission.enforcement.action, SecurityEnforcementAction::Ask);
    assert!(
        emission.enforcement.ask_id.is_some(),
        "an asking exec boundary must leave a pending ask on the ledger"
    );

    writer.flush().await;
    writer.shutdown_blocking();
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let pending: i64 = conn
        .query_row(
            "SELECT count(*) FROM security_ask_events WHERE status = 'pending'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pending, 1);
}

#[tokio::test]
async fn process_exec_boundary_honors_a_blocking_plugin() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 32).unwrap();
    let rules = exec_boundary_rules("allow");
    let mut plugin_policy = BTreeMap::new();
    plugin_policy.insert(
        "dummy_post_allow".to_string(),
        SecurityPluginConfig {
            mode: SecurityPluginMode::Block,
            detection_level: crate::net::policy_config::DetectionLevel::Critical,
        },
    );

    let emission = emit_process_exec_security_boundary(&writer, &rules, plugin_policy, exec_event(10, "echo hello"))
        .await
        .expect("boundary evaluates")
        .expect("boundary writes a primary row");

    assert_eq!(emission.enforcement.action, SecurityEnforcementAction::Block);
    assert_eq!(
        emission.enforcement.reason.as_deref(),
        Some("process exec blocked by plugin"),
        "the process rail must name itself the way the file rail does"
    );

    writer.flush().await;
    writer.shutdown_blocking();
}

fn allow_everything_file_rules() -> SecurityRuleSet {
    let profile = SecurityRuleProfile::parse_toml(
        r#"
[default.file]
name = "file"
action = "allow"
priority = "default"
match = "has(file.export.path)"
"#,
    )
    .expect("profile parses");
    SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::BuiltinDefault).expect("rules compile")
}

fn blocking_plugin_policy() -> BTreeMap<String, SecurityPluginConfig> {
    let mut policy = BTreeMap::new();
    policy.insert(
        "dummy_post_allow".to_string(),
        SecurityPluginConfig {
            mode: SecurityPluginMode::Block,
            detection_level: crate::net::policy_config::DetectionLevel::Critical,
        },
    );
    policy
}

fn file_export_event() -> SecurityEvent {
    SecurityEvent::new(RuntimeSecurityEventType::FileExport).with_file(FileSecurityEvent {
        export_path: Some("/workspace/x.txt".to_string()),
        ..Default::default()
    })
}

/// The plugin-aware emitters are the surface a custom plugin will eventually be
/// evaluated through, so a plugin verdict has to reach the caller. They used to
/// return a row count, which made "ran the plugin, ignored what it said" the
/// path of least resistance.
#[tokio::test]
async fn plugin_aware_emitter_returns_the_plugin_verdict() {
    let tmp = tempfile::tempdir().unwrap();
    let writer = capsem_logger::DbWriter::open(&tmp.path().join("session.db"), 32).unwrap();
    let event_id = SecurityEventId::new_uuid4();

    let emission = emit_matching_security_rules_with_plugins(
        &writer,
        event_id.clone(),
        RuntimeSecurityEventType::FileExport,
        &allow_everything_file_rules(),
        blocking_plugin_policy(),
        file_export_event(),
        current_unix_ms(),
    )
    .await
    .expect("emission succeeds");

    assert_eq!(
        emission.enforcement.action,
        SecurityEnforcementAction::Block,
        "a blocking plugin over an allowing rule must come back as a block"
    );
    assert_eq!(
        emission.enforcement.reason.as_deref(),
        Some("file.export blocked by plugin")
    );
    assert_eq!(emission.event_id, event_id);
    writer.shutdown_blocking();
}

#[test]
fn plugin_aware_blocking_emitter_returns_the_plugin_verdict() {
    let tmp = tempfile::tempdir().unwrap();
    let writer = capsem_logger::DbWriter::open(&tmp.path().join("session.db"), 32).unwrap();
    let event_id = SecurityEventId::new_uuid4();

    let emission = emit_matching_security_rules_with_plugins_blocking(
        &writer,
        event_id.clone(),
        RuntimeSecurityEventType::FileExport,
        &allow_everything_file_rules(),
        blocking_plugin_policy(),
        file_export_event(),
        current_unix_ms(),
    )
    .expect("emission succeeds");

    assert_eq!(emission.enforcement.action, SecurityEnforcementAction::Block);
    assert_eq!(emission.event_id, event_id);
    writer.shutdown_blocking();
}

#[tokio::test]
async fn plugin_aware_emitter_leaves_an_allowing_plugin_alone() {
    let tmp = tempfile::tempdir().unwrap();
    let writer = capsem_logger::DbWriter::open(&tmp.path().join("session.db"), 32).unwrap();
    let mut policy = BTreeMap::new();
    policy.insert(
        "dummy_post_allow".to_string(),
        SecurityPluginConfig {
            mode: SecurityPluginMode::Allow,
            detection_level: crate::net::policy_config::DetectionLevel::Informational,
        },
    );

    let emission = emit_matching_security_rules_with_plugins(
        &writer,
        SecurityEventId::new_uuid4(),
        RuntimeSecurityEventType::FileExport,
        &allow_everything_file_rules(),
        policy,
        file_export_event(),
        current_unix_ms(),
    )
    .await
    .expect("emission succeeds");

    assert_eq!(emission.enforcement.action, SecurityEnforcementAction::Allow);
    assert!(emission.enforcement.reason.is_none());
    writer.shutdown_blocking();
}

/// The published field table is what a rule author reads before writing a rule,
/// and the compiler now rejects anything outside it. A table that drifts from
/// the contract sends authors to rules that will not compile -- it listed a
/// `security` root that was never valid and omitted `ip`, `tcp`, and `udp` while
/// a shipped profile rule used all three.
#[test]
fn published_field_table_matches_the_cel_contract() {
    const POLICY_DOC: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../web/docs/src/content/docs/security/policy.md"
    ));

    let section = POLICY_DOC
        .split("## First-Party Fields")
        .nth(1)
        .expect("policy doc has a First-Party Fields section")
        .split("\n## ")
        .next()
        .expect("section ends at the next heading");

    // Only the Fields column of the table counts. The Root column and the
    // surrounding prose both carry backticks that are not field names.
    let documented = section
        .lines()
        .filter(|line| line.starts_with('|'))
        .filter_map(|line| line.split('|').nth(2))
        .flat_map(|cell| cell.split('`').skip(1).step_by(2))
        .collect::<std::collections::BTreeSet<_>>();
    let contract = SECURITY_EVENT_CEL_FIELDS
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();

    let undocumented = contract.difference(&documented).collect::<Vec<_>>();
    let invented = documented.difference(&contract).collect::<Vec<_>>();

    assert!(
        undocumented.is_empty(),
        "these CEL fields exist but the policy doc does not list them: {undocumented:?}"
    );
    assert!(
        invented.is_empty(),
        "the policy doc advertises fields the compiler rejects: {invented:?}"
    );
}
