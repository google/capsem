use super::*;

#[test]
fn denied_ask_resolution_blocks_like_block() {
    let decision = SecurityEnforcementDecision {
        action: SecurityEnforcementAction::Ask,
        rule_id: Some("profiles.rules.ask_openai".to_string()),
        rule_name: Some("ask_openai".to_string()),
        reason: None,
        ask_id: Some(SecurityEventId::parse("abcdef123456").unwrap()),
    };
    let denied = capsem_logger::SecurityAskEvent::pending(capsem_logger::SecurityAskPending {
        timestamp_unix_ms: 1_789_000_000_290,
        ask_id: "abcdef123456".to_string(),
        event_id: "aaaaaa111111".to_string(),
        event_type: RuntimeSecurityEventType::HttpRequest.as_str().to_string(),
        rule_id: "profiles.rules.ask_openai".to_string(),
        rule_name: "ask_openai".to_string(),
        rule_json: "{}".to_string(),
        event_json: "{}".to_string(),
    })
    .with_status(capsem_logger::SecurityAskStatus::Denied)
    .with_resolver("tester")
    .with_reason("denied for test");
    let resolved = decision.with_ask_resolution(&denied).unwrap();
    let event =
        SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http_request(HttpRequestSecurityEvent::new(
            "api.openai.com",
            Some(ProviderKind::OpenAi),
            http::HeaderMap::new(),
            None,
        ));

    assert_eq!(resolved.action, SecurityEnforcementAction::Block);
    assert_eq!(resolved.reason.as_deref(), Some("denied for test"));
    let error = materialize_http_request_for_upstream_after_enforcement(&event, &resolved)
        .expect_err("denied ask must block materialization");
    assert!(error.to_string().contains("profiles.rules.ask_openai"));
}

#[tokio::test]
async fn emit_file_security_write_and_rules_maps_created_file_to_create_root() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    let profile = SecurityRuleProfile::parse_toml(
        r#"
[profiles.rules.file_create_seen]
name = "file_create_seen"
action = "allow"
detection_level = "informational"
match = 'file.create.path == "/workspace/skills/foo.md" && file.create.name == "foo.md" && file.create.ext == "md"'
"#,
    )
    .unwrap();
    let rules =
        crate::net::policy_config::SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User).unwrap();

    let event_id = emit_file_security_write_and_rules(
        &writer,
        &rules,
        FileEvent {
            event_id: None,
            timestamp: SystemTime::now(),
            action: FileAction::Created,
            path: "/workspace/skills/foo.md".to_string(),
            size: Some(12),
            trace_id: Some("trace_file_create".to_string()),
            credential_ref: None,
        },
    )
    .await
    .expect("file event must receive id");
    writer.shutdown_blocking();

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let fs_event_id: String = conn
        .query_row("SELECT event_id FROM fs_events", [], |row| row.get(0))
        .unwrap();
    let rule_row: (String, String) = conn
        .query_row("SELECT event_id, rule_id FROM security_rule_events", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();
    assert_eq!(fs_event_id, event_id.as_str());
    assert_eq!(rule_row.0, event_id.as_str());
    assert_eq!(rule_row.1, "profiles.rules.file_create_seen");
}

#[tokio::test]
async fn emit_explicit_file_security_events_map_import_export_and_read_roots() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 32).unwrap();
    let profile = SecurityRuleProfile::parse_toml(
        r#"
[profiles.rules.file_import_seen]
name = "file_import_seen"
action = "allow"
detection_level = "informational"
match = 'file.import.path.endsWith("input.txt") && file.import.mime_type == "text/plain" && file.import.content.contains("incoming")'

[profiles.rules.file_export_seen]
name = "file_export_seen"
action = "allow"
detection_level = "informational"
match = 'file.export.name == "output.json" && file.export.ext == "json" && file.export.content.contains("ok")'

[profiles.rules.file_read_seen]
name = "file_read_seen"
action = "allow"
detection_level = "informational"
match = 'file.read.path.contains("skills/") && file.read.ext == "md" && file.read.content.contains("Development Sprint")'
"#,
    )
    .unwrap();
    let rules =
        crate::net::policy_config::SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User).unwrap();

    for event in [
        ExplicitFileSecurityEvent {
            action: FileAction::Imported,
            path: "/workspace/input.txt".to_string(),
            size: Some(8),
            content: Some("incoming".to_string()),
            mime_type: Some("text/plain".to_string()),
            trace_id: Some("trace_file_import".to_string()),
            credential_ref: None,
        },
        ExplicitFileSecurityEvent {
            action: FileAction::Exported,
            path: "/workspace/output.json".to_string(),
            size: Some(11),
            content: Some(r#"{"ok":true}"#.to_string()),
            mime_type: Some("application/json".to_string()),
            trace_id: Some("trace_file_export".to_string()),
            credential_ref: None,
        },
        ExplicitFileSecurityEvent {
            action: FileAction::Read,
            path: "/workspace/skills/skill.md".to_string(),
            size: Some(20),
            content: Some("Development Sprint".to_string()),
            mime_type: Some("text/markdown".to_string()),
            trace_id: Some("trace_file_read".to_string()),
            credential_ref: None,
        },
    ] {
        emit_explicit_file_security_write_and_rules(&writer, &rules, event)
            .await
            .expect("explicit file event must receive id");
    }
    writer.shutdown_blocking();

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let actions = conn
        .prepare("SELECT action FROM fs_events ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(actions, vec!["import", "export", "read"]);

    let rules = conn
        .prepare("SELECT rule_id, event_type, event_json FROM security_rule_events ORDER BY id")
        .unwrap()
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        rules.iter().map(|row| row.0.as_str()).collect::<Vec<_>>(),
        vec![
            "profiles.rules.file_import_seen",
            "profiles.rules.file_export_seen",
            "profiles.rules.file_read_seen",
        ]
    );
    assert_eq!(rules[0].1, "file.import");
    assert_eq!(rules[1].1, "file.export");
    assert_eq!(rules[2].1, "file.event");
    assert!(rules[0].2.contains(r#""import_content":"incoming""#));
    assert!(rules[1].2.contains(r#""export_mime_type":"application/json""#));
    assert!(rules[2].2.contains(r#""read_content":"Development Sprint""#));
}

#[tokio::test]
async fn emit_process_exec_and_complete_rules_share_exec_event_id() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    let profile = SecurityRuleProfile::parse_toml(
        r#"
[profiles.rules.process_exec_seen]
name = "process_exec_seen"
action = "allow"
detection_level = "informational"
match = 'process.command.contains("python")'

[profiles.rules.process_complete_seen]
name = "process_complete_seen"
action = "allow"
detection_level = "low"
match = 'process.exec.id == "42" && process.exec.exit_code == "0" && process.exec.stdout.contains("ok")'
"#,
    )
    .unwrap();
    let rules =
        crate::net::policy_config::SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User).unwrap();

    let event_id = emit_process_exec_security_boundary(
        &writer,
        &rules,
        BTreeMap::new(),
        ExecEvent {
            event_id: None,
            timestamp: SystemTime::now(),
            exec_id: 42,
            command: "python main.py".to_string(),
            source: "api".to_string(),
            trace_id: Some("trace_exec".to_string()),
            process_name: None,
            credential_ref: None,
        },
    )
    .await
    .expect("exec boundary evaluates")
    .expect("exec event must receive id")
    .event_id;
    emit_process_complete_security_write_and_rules(
        &writer,
        &rules,
        event_id.clone(),
        ExecEventComplete {
            exec_id: 42,
            exit_code: 0,
            duration_ms: 12,
            stdout_preview: Some("ok".to_string()),
            stderr_preview: None,
            stdout_bytes: 2,
            stderr_bytes: 0,
            pid: Some(1000),
        },
    )
    .await
    .expect("exec complete must reuse primary id");
    writer.shutdown_blocking();

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let exec_event_id: String = conn
        .query_row("SELECT event_id FROM exec_events WHERE exec_id = 42", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(exec_event_id, event_id.as_str());
    let rows: Vec<(String, String, String)> = {
        let mut stmt = conn
            .prepare(
                "SELECT event_id, event_type, rule_id
                 FROM security_rule_events ORDER BY rule_id",
            )
            .unwrap();
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    assert_eq!(
        rows,
        vec![
            (
                event_id.as_str().to_string(),
                "process.exec_complete".to_string(),
                "profiles.rules.process_complete_seen".to_string()
            ),
            (
                event_id.as_str().to_string(),
                "process.exec".to_string(),
                "profiles.rules.process_exec_seen".to_string()
            ),
        ]
    );
}

#[tokio::test]
async fn emit_substitution_security_write_and_rules_keeps_ref_without_fake_root() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    let rules = SecurityRuleSet::new(Vec::new());
    let credential_ref = capsem_logger::credential_reference("openai", "sk-test-secret");

    let event_id = emit_substitution_security_write_and_rules(
        &writer,
        &rules,
        SubstitutionEvent {
            event_id: None,
            timestamp: SystemTime::now(),
            material_class: "credential".to_string(),
            source: "http.response".to_string(),
            event_type: Some("http.request".to_string()),
            algorithm: "blake3".to_string(),
            substitution_ref: credential_ref.clone(),
            outcome: "captured".to_string(),
            provider: Some("openai".to_string()),
            confidence: None,
            trace_id: Some("trace_credential".to_string()),
            context_json: None,
        },
    )
    .await
    .expect("substitution event must receive id");
    writer.shutdown_blocking();

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let substitution_event_id: String = conn
        .query_row("SELECT event_id FROM substitution_events", [], |row| row.get(0))
        .unwrap();
    let persisted_ref: String = conn
        .query_row("SELECT substitution_ref FROM substitution_events", [], |row| row.get(0))
        .unwrap();
    let rule_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM security_rule_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(substitution_event_id, event_id.as_str());
    assert_eq!(persisted_ref, credential_ref);
    assert_eq!(rule_count, 0);
}

#[tokio::test]
async fn emit_matching_security_rules_writes_no_rows_for_non_match() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    let profile = SecurityRuleProfile::parse_toml(
        r#"
[profiles.rules.http_block]
name = "http_block"
action = "block"
match = 'http.host.contains("openai.com")'
"#,
    )
    .unwrap();
    let rules =
        crate::net::policy_config::SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User).unwrap();
    let event_id = emit_security_write(&writer, net_write(None))
        .await
        .expect("primary HTTP event must receive an id");
    let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http(HttpSecurityEvent {
        host: Some("example.com".into()),
        ..Default::default()
    });

    let emitted = emit_matching_security_rules(
        &writer,
        event_id,
        RuntimeSecurityEventType::HttpRequest,
        &rules,
        &event,
        1_789_000_000_300,
    )
    .await
    .unwrap();
    writer.shutdown_blocking();

    assert_eq!(emitted, 0);
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM security_rule_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn http_host_matching_is_case_insensitive() {
    // Hostnames are case-insensitive (the DNS path already lowercases qnames).
    // A block rule on a lowercase host must still fire when a guest sends the
    // request with a mixed-case Host header, otherwise the block is trivially
    // evaded with `Host: API.Evil.Com`.
    let profile = SecurityRuleProfile::parse_toml(
        r#"
[profiles.rules.block_evil]
name = "block_evil"
action = "block"
match = 'http.host == "api.evil.com"'
"#,
    )
    .unwrap();
    let rules =
        crate::net::policy_config::SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User).unwrap();

    let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http(HttpSecurityEvent {
        host: Some("API.Evil.Com".into()),
        ..Default::default()
    });

    let boundary = evaluate_security_boundary(&rules, std::collections::BTreeMap::new(), event).unwrap();
    assert_eq!(
        boundary.enforcement.action,
        SecurityEnforcementAction::Block,
        "mixed-case Host must not evade a lowercase host block rule"
    );
}
