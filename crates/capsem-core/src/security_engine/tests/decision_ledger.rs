use super::*;
use crate::security_engine::forensics::SecurityRuleTraceLabels;

#[tokio::test]
async fn emit_security_rule_match_writes_forensic_ledger_row() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    let profile = SecurityRuleProfile::parse_toml(
        r#"
[profiles.rules.block_openai]
name = "openai_api_block"
action = "block"
detection_level = "critical"
match = 'http.host.matches("(^|.*\.)openai\.com$")'
priority = 10
reason = "corp block"
"#,
    )
    .unwrap();
    let rule_set = SecurityRuleProfile::compile(&profile, SecurityRuleSource::User).unwrap();
    let rule = rule_set
        .iter()
        .find(|rule| rule.rule_id == "profiles.rules.block_openai")
        .unwrap();
    let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest)
        .with_trace_id("trace_deadbeef")
        .with_http(HttpSecurityEvent {
            host: Some("api.openai.com".into()),
            method: Some("POST".into()),
            path: Some("/v1/chat/completions".into()),
            query: None,
            status: None,
            body: Some("{\"model\":\"gpt-4.1\"}".into()),
        })
        .with_ip(IpSecurityEvent {
            value: Some("203.0.113.10".into()),
            version: Some("4".into()),
        })
        .with_tcp(TcpSecurityEvent {
            port: Some("443".into()),
        })
        .with_credential_observations(vec![CredentialObservation {
            provider: CredentialProvider::OpenAi,
            raw_value: "sk-live-should-not-appear".into(),
            source: "http.request.header.authorization".into(),
            event_type: Some("http.request".into()),
            trace_id: Some("trace_deadbeef".into()),
            context_json: None,
        }]);

    emit_security_rule_match(
        &writer,
        SecurityEventId::parse("abcdef123456").unwrap(),
        RuntimeSecurityEventType::HttpRequest,
        rule,
        &event,
        1_789_000_000_000,
    )
    .await
    .unwrap();
    writer.shutdown_blocking();

    let reader = capsem_logger::DbReader::open(&db_path).unwrap();
    let rows = reader.recent_security_rule_events(10).unwrap();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.event_id, "abcdef123456");
    assert_eq!(row.event_type, "http.request");
    assert_eq!(row.rule_id, "profiles.rules.block_openai");
    assert_eq!(row.rule_action, capsem_logger::SecurityRuleAction::Block);
    assert_eq!(row.detection_level, capsem_logger::SecurityDetectionLevel::Critical);
    assert!(row.rule_json.contains("openai_api_block"));
    assert!(row.event_json.contains("api.openai.com"));
    let event_json: serde_json::Value = serde_json::from_str(&row.event_json).unwrap();
    assert_eq!(event_json["event_type"], "http.request");
    assert_eq!(event_json["http"]["host"], "api.openai.com");
    assert_eq!(event_json["ip"]["value"], "203.0.113.10");
    assert_eq!(event_json["ip"]["version"], "4");
    assert_eq!(event_json["tcp"]["port"], "443");
    assert!(row.event_json.contains("credential:blake3:"));
    assert!(
        !row.event_json.contains("sk-live-should-not-appear"),
        "forensic event payload must not store raw credential observations"
    );
}

#[test]
fn security_rule_trace_labels_are_low_cardinality_rule_fields() {
    let profile = SecurityRuleProfile::parse_toml(
        r#"
[profiles.rules.block_openai]
name = "openai_api_block"
action = "block"
detection_level = "critical"
match = 'http.host == "api.openai.com"'
"#,
    )
    .unwrap();
    let rules = SecurityRuleProfile::compile(&profile, SecurityRuleSource::User).unwrap();
    let rule = rules
        .iter()
        .find(|rule| rule.rule_id == "profiles.rules.block_openai")
        .unwrap();

    let labels = SecurityRuleTraceLabels::from_rule(rule);

    assert_eq!(labels.rule_id, "profiles.rules.block_openai");
    assert_eq!(labels.rule_name, "openai_api_block");
    assert_eq!(labels.rule_action, "block");
    assert_eq!(labels.rule_detection_level, "critical");
    assert_eq!(labels.provider, "profiles");
}

#[tokio::test]
async fn primary_event_and_rule_ledger_share_event_id() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    let profile = SecurityRuleProfile::parse_toml(
        r#"
[profiles.rules.file_skill_loaded]
name = "file_skill_loaded"
action = "allow"
detection_level = "informational"
match = 'file.read.path.contains("skills/") && file.read.name.endsWith(".md")'
"#,
    )
    .unwrap();
    let rule_set = SecurityRuleProfile::compile(&profile, SecurityRuleSource::User).unwrap();
    let rule = rule_set
        .iter()
        .find(|rule| rule.rule_id == "profiles.rules.file_skill_loaded")
        .unwrap();

    let event_id = emit_security_write(&writer, file_write(None))
        .await
        .expect("file event must receive a primary event id");
    let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest)
        .with_trace_id("trace_file_skill")
        .with_file(FileSecurityEvent {
            read_path: Some("/root/.codex/skills/example/SKILL.md".into()),
            read_name: Some("SKILL.md".into()),
            read_ext: Some("md".into()),
            read_mime_type: Some("text/markdown".into()),
            read_content: Some("# skill".into()),
            ..Default::default()
        });

    emit_security_rule_match(
        &writer,
        event_id.clone(),
        RuntimeSecurityEventType::FileEvent,
        rule,
        &event,
        1_789_000_000_100,
    )
    .await
    .unwrap();
    writer.shutdown_blocking();

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let fs_event_id: String = conn
        .query_row("SELECT event_id FROM fs_events", [], |row| row.get(0))
        .unwrap();
    let rule_event_id: String = conn
        .query_row("SELECT event_id FROM security_rule_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(fs_event_id, event_id.as_str());
    assert_eq!(rule_event_id, event_id.as_str());
}

#[tokio::test]
async fn emit_matching_security_rules_writes_all_matches_with_primary_event_id() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    let profile = SecurityRuleProfile::parse_toml(
        r#"
[profiles.rules.http_observed]
name = "http_observed"
action = "allow"
detection_level = "informational"
match = 'http.host.contains("openai.com")'

[profiles.rules.http_block]
name = "http_block"
action = "block"
detection_level = "critical"
match = 'http.path.startsWith("/v1/")'
"#,
    )
    .unwrap();
    let rules =
        crate::net::policy_config::SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User).unwrap();

    let event_id = emit_security_write(&writer, net_write(None))
        .await
        .expect("primary HTTP event must receive an id");
    let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest)
        .with_trace_id("trace_http_rules")
        .with_http(HttpSecurityEvent {
            host: Some("api.openai.com".into()),
            method: Some("POST".into()),
            path: Some("/v1/responses".into()),
            query: None,
            status: Some("200".into()),
            body: None,
        });

    let emitted = emit_matching_security_rules(
        &writer,
        event_id.clone(),
        RuntimeSecurityEventType::HttpRequest,
        &rules,
        &event,
        1_789_000_000_200,
    )
    .await
    .unwrap();
    writer.shutdown_blocking();

    assert_eq!(emitted, 2);
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let net_event_id: String = conn
        .query_row("SELECT event_id FROM net_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(net_event_id, event_id.as_str());
    let rows: Vec<(String, String, String)> = {
        let mut stmt = conn
            .prepare(
                "SELECT event_id, rule_id, detection_level
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
                "profiles.rules.http_block".to_string(),
                "critical".to_string()
            ),
            (
                event_id.as_str().to_string(),
                "profiles.rules.http_observed".to_string(),
                "informational".to_string()
            ),
        ]
    );
}

#[tokio::test]
async fn emit_matching_security_rules_with_decision_uses_same_evaluation_as_ledger() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    let profile = SecurityRuleProfile::parse_toml(
        r#"
[corp.rules.block_openai]
name = "block_openai"
action = "block"
priority = -10
reason = "corp block"
match = 'http.host == "api.openai.com"'

[profiles.rules.detect_openai]
name = "detect_openai"
action = "allow"
detection_level = "high"
priority = 10
match = 'http.host == "api.openai.com"'

[profiles.rules.ask_model]
name = "ask_model"
action = "ask"
priority = 20
match = 'model.provider == "openai"'
"#,
    )
    .unwrap();
    let rules = SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User).unwrap();
    let event_id = emit_security_write(&writer, net_write(None))
        .await
        .expect("primary HTTP event must receive an id");
    let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http(HttpSecurityEvent {
        host: Some("api.openai.com".into()),
        method: Some("POST".into()),
        path: Some("/v1/responses".into()),
        ..Default::default()
    });

    let emission = emit_matching_security_rules_with_decision(
        &writer,
        event_id.clone(),
        RuntimeSecurityEventType::HttpRequest,
        &rules,
        &event,
        1_789_000_000_250,
    )
    .await
    .unwrap();
    writer.shutdown_blocking();

    assert_eq!(emission.emitted, 2);
    assert_eq!(emission.enforcement.action, SecurityEnforcementAction::Block);
    assert_eq!(emission.enforcement.rule_id.as_deref(), Some("corp.rules.block_openai"));
    assert_eq!(emission.enforcement.reason.as_deref(), Some("corp block"));

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let rows: Vec<(String, String)> = {
        let mut stmt = conn
            .prepare("SELECT rule_id, rule_action FROM security_rule_events ORDER BY rule_id")
            .unwrap();
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    assert_eq!(
        rows,
        vec![
            ("corp.rules.block_openai".to_string(), "block".to_string()),
            ("profiles.rules.detect_openai".to_string(), "allow".to_string()),
        ],
        "the decision must be derived from the same matches that were ledgered"
    );

    let decision_rows: Vec<(String, String, String, String, String)> = {
        let mut stmt = conn
            .prepare(
                "SELECT actor, previous_decision, requested_decision, effective_decision, rule_id
                 FROM security_decision_events
                 ORDER BY id",
            )
            .unwrap();
        stmt.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
    };
    assert_eq!(
        decision_rows,
        vec![(
            "corp.rules.block_openai".to_string(),
            "allow".to_string(),
            "block".to_string(),
            "block".to_string(),
            "corp.rules.block_openai".to_string(),
        )],
        "only the first matching enforcement rule produces a decision transition"
    );
}

#[tokio::test]
async fn decision_ledger_uses_the_same_first_match_as_enforcement() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    let rules = security_rule_set(
        r#"
[profiles.rules.allow_preferred]
name = "allow_preferred"
action = "allow"
priority = 10
match = 'http.host == "api.example.com"'

[profiles.rules.block_weaker]
name = "block_weaker"
action = "block"
priority = 20
match = 'http.host == "api.example.com"'
"#,
    );
    let event_id = emit_security_write(&writer, net_write(None))
        .await
        .expect("primary HTTP event must receive an id");
    let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http(HttpSecurityEvent {
        host: Some("api.example.com".into()),
        ..Default::default()
    });

    let emission = emit_matching_security_rules_with_decision(
        &writer,
        event_id,
        RuntimeSecurityEventType::HttpRequest,
        &rules,
        &event,
        1_789_000_000_255,
    )
    .await
    .unwrap();
    writer.flush().await;
    writer.shutdown_blocking();

    assert_eq!(emission.enforcement.action, SecurityEnforcementAction::Allow);
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let matched_rules: i64 = conn
        .query_row("SELECT count(*) FROM security_rule_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(matched_rules, 2, "every matched rule remains forensic evidence");
    let decisions: Vec<(String, String)> = {
        let mut stmt = conn
            .prepare(
                "SELECT rule_id, effective_decision
                 FROM security_decision_events ORDER BY id",
            )
            .unwrap();
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    assert_eq!(
        decisions,
        vec![("profiles.rules.allow_preferred".to_string(), "allow".to_string())]
    );
}

#[tokio::test]
async fn emit_matching_security_rules_with_decision_defaults_to_allow_without_enforcement_match() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    let rules = security_rule_set(
        r#"
[profiles.rules.detect_skill]
name = "detect_skill"
action = "postprocess"
detection_level = "informational"
match = 'file.read.name == "SKILL.md"'
"#,
    );
    let event_id = emit_security_write(&writer, file_write(None))
        .await
        .expect("primary file event must receive an id");
    let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_file(FileSecurityEvent {
        read_name: Some("SKILL.md".into()),
        ..Default::default()
    });

    let emission = emit_matching_security_rules_with_decision(
        &writer,
        event_id,
        RuntimeSecurityEventType::FileEvent,
        &rules,
        &event,
        1_789_000_000_260,
    )
    .await
    .unwrap();
    writer.shutdown_blocking();

    assert_eq!(emission.emitted, 1);
    assert_eq!(emission.enforcement, SecurityEnforcementDecision::allow());
}

#[tokio::test]
async fn default_rules_do_not_override_specific_enforcement_decisions() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    let rules = security_rule_set(
        r#"
[profiles.rules.allow_local_fixture]
name = "allow_local_fixture"
action = "allow"
priority = 10
detection_level = "informational"
reason = "Hermetic fixture endpoint is explicitly allowed."
match = 'http.host == "127.0.0.1" && tcp.port == "3713"'

[default.000_local_network]
name = "local_network"
action = "ask"
priority = "default"
reason = "Default ask before local network access."
match = 'ip.value == "127.0.0.1" || http.host == "127.0.0.1"'
"#,
    );
    let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest)
        .with_http(HttpSecurityEvent {
            host: Some("127.0.0.1".into()),
            method: Some("POST".into()),
            path: Some("/v1/chat/completions".into()),
            ..Default::default()
        })
        .with_ip(IpSecurityEvent {
            value: Some("127.0.0.1".into()),
            version: Some("4".into()),
        })
        .with_tcp(TcpSecurityEvent {
            port: Some("3713".into()),
        });

    let boundary = evaluate_security_boundary(&rules, BTreeMap::new(), event.clone()).unwrap();
    assert_eq!(boundary.matched_rule_count, 2);
    assert_eq!(boundary.enforcement.action, SecurityEnforcementAction::Allow);
    assert_eq!(
        boundary.enforcement.rule_id.as_deref(),
        Some("profiles.rules.allow_local_fixture")
    );
    assert_eq!(boundary.event.decision.effective, SecurityDecisionKind::Allow);

    let event_id = emit_security_write(&writer, net_write(None))
        .await
        .expect("primary HTTP event must receive an id");
    let emission = emit_matching_security_rules_with_decision(
        &writer,
        event_id,
        RuntimeSecurityEventType::HttpRequest,
        &rules,
        &event,
        1_789_000_000_265,
    )
    .await
    .unwrap();
    writer.shutdown_blocking();

    assert_eq!(emission.emitted, 2);
    assert_eq!(emission.enforcement.action, SecurityEnforcementAction::Allow);
    assert_eq!(
        emission.enforcement.rule_id.as_deref(),
        Some("profiles.rules.allow_local_fixture")
    );

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let rule_rows: Vec<(String, String)> = {
        let mut stmt = conn
            .prepare("SELECT rule_id, rule_action FROM security_rule_events ORDER BY rule_id")
            .unwrap();
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    assert_eq!(
        rule_rows,
        vec![
            ("profiles.rules.allow_local_fixture".to_string(), "allow".to_string(),),
            (
                "profiles.rules.default_000_local_network".to_string(),
                "ask".to_string(),
            ),
        ],
        "the default catchall remains visible in the rule ledger"
    );
    let decision_rows: Vec<(String, String, String)> = {
        let mut stmt = conn
            .prepare(
                "SELECT rule_id, requested_decision, effective_decision
                 FROM security_decision_events ORDER BY id",
            )
            .unwrap();
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    assert_eq!(
        decision_rows,
        vec![(
            "profiles.rules.allow_local_fixture".to_string(),
            "allow".to_string(),
            "allow".to_string(),
        )],
        "the default ask must not appear as an effective decision after a specific allow"
    );
}

#[tokio::test]
async fn ask_enforcement_writes_pending_and_resolution_controls_materialization() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    let rules = security_rule_set(
        r#"
[profiles.rules.ask_openai]
name = "ask_openai"
action = "ask"
reason = "manual approval required"
match = 'http.host == "api.openai.com"'
"#,
    );
    let event_id = emit_security_write(&writer, net_write(None))
        .await
        .expect("primary HTTP event must receive an id");
    let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest)
        .with_trace_id("trace_ask")
        .with_http(HttpSecurityEvent {
            host: Some("api.openai.com".into()),
            method: Some("POST".into()),
            path: Some("/v1/responses".into()),
            ..Default::default()
        })
        .with_http_request(HttpRequestSecurityEvent::new(
            "api.openai.com",
            Some(ProviderKind::OpenAi),
            http::HeaderMap::new(),
            None,
        ));

    let emission = emit_matching_security_rules_with_decision(
        &writer,
        event_id.clone(),
        RuntimeSecurityEventType::HttpRequest,
        &rules,
        &event,
        1_789_000_000_270,
    )
    .await
    .unwrap();

    assert_eq!(emission.emitted, 1);
    assert_eq!(emission.enforcement.action, SecurityEnforcementAction::Ask);
    let ask_id = emission
        .enforcement
        .ask_id
        .clone()
        .expect("ask decision must return ask_id");
    let ask_rule = rules
        .rules()
        .iter()
        .find(|rule| rule.rule_id == "profiles.rules.ask_openai")
        .expect("ask rule must compile");
    let pending = security_ask_pending_event(
        ask_id.clone(),
        event_id.clone(),
        RuntimeSecurityEventType::HttpRequest,
        ask_rule,
        &event,
        1_789_000_000_270,
    )
    .unwrap();
    let unresolved = emission.enforcement.with_ask_resolution(&pending);
    assert!(unresolved.unwrap_err().to_string().contains("still pending"));
    let pending_error = materialize_http_request_for_upstream_after_enforcement(&event, &emission.enforcement)
        .expect_err("pending ask must block materialization");
    assert!(pending_error.to_string().contains("ask"));

    emit_security_ask_resolution(
        &writer,
        &pending,
        capsem_logger::SecurityAskStatus::Approved,
        "tester",
        Some("approved for test".to_string()),
        1_789_000_000_280,
    )
    .await
    .unwrap();
    writer.shutdown_blocking();

    let reader = capsem_logger::DbReader::open(&db_path).unwrap();
    let ask_rows = reader.recent_security_ask_events(10).unwrap();
    assert_eq!(ask_rows.len(), 2);
    let latest = reader
        .latest_security_ask_event(ask_id.as_str())
        .unwrap()
        .expect("resolution row must exist");
    assert_eq!(latest.status, capsem_logger::SecurityAskStatus::Approved);
    assert_eq!(latest.resolver.as_deref(), Some("tester"));
    assert_eq!(latest.event_id, event_id.as_str());
    assert_eq!(latest.rule_id, "profiles.rules.ask_openai");

    let approved = emission.enforcement.with_ask_resolution(&latest).unwrap();
    assert_eq!(approved.action, SecurityEnforcementAction::Allow);
    materialize_http_request_for_upstream_after_enforcement(&event, &approved)
        .expect("approved ask should materialize like allow");

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let ledger_rule_id: String = conn
        .query_row("SELECT rule_id FROM security_rule_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(ledger_rule_id, "profiles.rules.ask_openai");
}

#[tokio::test]
async fn session_db_regenerates_rule_enforcement_detection_and_ask_story() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    let github_rules = security_rule_set(
        r#"
[corp.rules.github_block]
name = "github_block"
action = "block"
detection_level = "critical"
priority = -10
reason = "corp block"
match = 'http.host == "github.com"'

[profiles.rules.github_detect]
name = "github_detect"
action = "allow"
detection_level = "high"
match = 'http.host == "github.com"'

[profiles.rules.github_postprocess]
name = "github_postprocess"
action = "postprocess"
detection_level = "informational"
match = 'http.host == "github.com"'
"#,
    );
    let github_event_id = emit_security_write(&writer, net_write(None))
        .await
        .expect("primary HTTP event must receive an id");
    let github_event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest)
        .with_trace_id("trace_github")
        .with_http(HttpSecurityEvent {
            host: Some("github.com".into()),
            method: Some("GET".into()),
            path: Some("/settings/tokens".into()),
            ..Default::default()
        });

    let github_emission = emit_matching_security_rules_with_decision(
        &writer,
        github_event_id.clone(),
        RuntimeSecurityEventType::HttpRequest,
        &github_rules,
        &github_event,
        1_789_000_000_310,
    )
    .await
    .unwrap();
    assert_eq!(github_emission.emitted, 3);
    assert_eq!(github_emission.enforcement.action, SecurityEnforcementAction::Block);
    assert_eq!(
        github_emission.enforcement.rule_id.as_deref(),
        Some("corp.rules.github_block")
    );

    let ask_rules = security_rule_set(
        r#"
[profiles.rules.ask_openai]
name = "ask_openai"
action = "ask"
reason = "manual approval required"
match = 'http.host == "api.openai.com"'
"#,
    );
    let ask_event_id = emit_security_write(&writer, net_write(None))
        .await
        .expect("primary HTTP event must receive an id");
    let ask_event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest)
        .with_trace_id("trace_openai_ask")
        .with_http(HttpSecurityEvent {
            host: Some("api.openai.com".into()),
            method: Some("POST".into()),
            path: Some("/v1/responses".into()),
            ..Default::default()
        });

    let ask_emission = emit_matching_security_rules_with_decision(
        &writer,
        ask_event_id.clone(),
        RuntimeSecurityEventType::HttpRequest,
        &ask_rules,
        &ask_event,
        1_789_000_000_320,
    )
    .await
    .unwrap();
    let ask_id = ask_emission
        .enforcement
        .ask_id
        .clone()
        .expect("ask decision must return ask_id");
    let ask_rule = ask_rules
        .rules()
        .iter()
        .find(|rule| rule.rule_id == "profiles.rules.ask_openai")
        .expect("ask rule must compile");
    let pending = security_ask_pending_event(
        ask_id.clone(),
        ask_event_id.clone(),
        RuntimeSecurityEventType::HttpRequest,
        ask_rule,
        &ask_event,
        1_789_000_000_320,
    )
    .unwrap();
    emit_security_ask_resolution(
        &writer,
        &pending,
        capsem_logger::SecurityAskStatus::Denied,
        "tester",
        Some("denied for test".to_string()),
        1_789_000_000_330,
    )
    .await
    .unwrap();
    writer.shutdown_blocking();

    let reader = capsem_logger::DbReader::open(&db_path).unwrap();
    let rows = reader.recent_security_rule_events(10).unwrap();
    assert_eq!(rows.len(), 4);

    let postprocess_row = rows
        .iter()
        .find(|row| row.rule_id == "profiles.rules.github_postprocess")
        .expect("postprocess detection rule row must be present");
    assert_eq!(postprocess_row.event_id, github_event_id.as_str());
    assert_eq!(postprocess_row.event_type, "http.request");
    assert_eq!(
        postprocess_row.rule_action,
        capsem_logger::SecurityRuleAction::Postprocess
    );
    assert_eq!(
        postprocess_row.detection_level,
        capsem_logger::SecurityDetectionLevel::Informational
    );
    let postprocess_rule: serde_json::Value = serde_json::from_str(&postprocess_row.rule_json).unwrap();
    assert_eq!(postprocess_rule["provider"], "profiles");
    assert_eq!(postprocess_rule["rule_action"], "postprocess");
    assert_eq!(postprocess_rule["detection_level"], "informational");
    assert!(postprocess_rule.get("plugin").is_none());
    let postprocess_event: serde_json::Value = serde_json::from_str(&postprocess_row.event_json).unwrap();
    assert_eq!(postprocess_event["event_type"], "http.request");
    assert_eq!(postprocess_event["http"]["host"], "github.com");

    let block_row = rows
        .iter()
        .find(|row| row.rule_id == "corp.rules.github_block")
        .expect("enforcement block row must be present");
    assert_eq!(block_row.rule_action, capsem_logger::SecurityRuleAction::Block);
    assert_eq!(
        block_row.detection_level,
        capsem_logger::SecurityDetectionLevel::Critical
    );
    let block_rule: serde_json::Value = serde_json::from_str(&block_row.rule_json).unwrap();
    assert_eq!(block_rule["reason"], "corp block");
    assert_eq!(block_rule["priority"], -10);

    let detect_row = rows
        .iter()
        .find(|row| row.rule_id == "profiles.rules.github_detect")
        .expect("detection row must be present");
    assert_eq!(detect_row.detection_level, capsem_logger::SecurityDetectionLevel::High);

    let ask_rows = reader.recent_security_ask_events(10).unwrap();
    assert_eq!(ask_rows.len(), 2);
    assert_eq!(ask_rows[0].status, capsem_logger::SecurityAskStatus::Denied);
    assert_eq!(ask_rows[0].ask_id, ask_id.as_str());
    assert_eq!(ask_rows[0].event_id, ask_event_id.as_str());
    assert_eq!(ask_rows[0].rule_id, "profiles.rules.ask_openai");
    assert_eq!(ask_rows[0].resolver.as_deref(), Some("tester"));
    assert_eq!(ask_rows[1].status, capsem_logger::SecurityAskStatus::Pending);

    let stats = reader.security_rule_stats().unwrap();
    assert_eq!(stats.total, 4);
    assert!(stats
        .by_action
        .iter()
        .any(|entry| entry.rule_action == "block" && entry.count == 1));
    assert!(stats
        .by_action
        .iter()
        .any(|entry| entry.rule_action == "postprocess" && entry.count == 1));
    assert!(stats
        .by_rule
        .iter()
        .any(|entry| entry.rule_id == "profiles.rules.github_postprocess"
            && entry.detection_level == "informational"
            && entry.latest_event_id == github_event_id.as_str()));
}
