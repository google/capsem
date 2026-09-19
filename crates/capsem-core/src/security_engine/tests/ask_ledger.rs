//! The ask lifecycle: a pending ask blocks materialization, its resolution
//! decides it, and the event it asked about is archived once.

use super::decision_ledger::archived_payload_of;
use super::*;

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
    let unresolved =
        emission
            .enforcement
            .with_ask_resolution(&pending.ask_id, pending.status, pending.reason.as_deref());
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

    let approved = emission
        .enforcement
        .with_ask_resolution(&latest.ask_id, latest.status, latest.reason.as_deref())
        .unwrap();
    assert_eq!(approved.action, SecurityEnforcementAction::Allow);
    materialize_http_request_for_upstream_after_enforcement(&event, &approved)
        .expect("approved ask should materialize like allow");

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let ledger_rule_id: String = conn
        .query_row("SELECT rule_id FROM security_rule_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(ledger_rule_id, "profiles.rules.ask_openai");
    drop(conn);

    // The asked-about event is archived once for the ask, not once per
    // lifecycle row: the pending row and its resolution name the same event,
    // and the resolution carries the pending row's payload unchanged.
    let asked = archived_payload_of(&db_path, "security_ask_events", event_id.as_str()).await;
    let asked: serde_json::Value = serde_json::from_str(&asked).unwrap();
    assert_eq!(asked["event_type"], "http.request");
    let archived: i64 = rusqlite::Connection::open(&db_path)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM event_body_blobs WHERE source_table = 'security_ask_events'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(archived, 1, "one ask, one archived event");
}
