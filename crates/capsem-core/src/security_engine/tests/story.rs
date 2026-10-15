use super::*;

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
    let postprocess_payload = archived_payload(&db_path, github_event_id.as_str()).await;
    let postprocess_event: serde_json::Value = serde_json::from_str(&postprocess_payload).unwrap();
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

    let security = capsem_logger::DbHandle::open_external_reader(&db_path)
        .unwrap()
        .ledger_counters()
        .await
        .unwrap()
        .security
        .clone();
    assert_eq!(security.matches, 4);
    assert_eq!(security.by_action.get("block"), Some(&1));
    assert_eq!(security.by_action.get("postprocess"), Some(&1));
    let postprocess = &security.by_rule["profiles.rules.github_postprocess"]["postprocess"]["informational"];
    assert_eq!(postprocess.latest_event_id, github_event_id.as_str());
}
