use super::*;

#[test]
fn plugin_only_accounting_borrows_evidence_without_duplicating_payload() {
    let mut event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http(HttpSecurityEvent {
        body: Some("private payload marker".into()),
        path: Some("/private/path".into()),
        ..Default::default()
    });
    event.record_plugin_execution(SecurityPluginExecution {
        plugin_id: "pii_filter".into(),
        stage: SecurityPluginStage::Preprocess,
        applied: true,
        duration_us: 9,
    });
    let write = plugin_execution_write(SecurityEventId::new_uuid4(), event.event_type, &event, 100)
        .unwrap()
        .unwrap();
    let WriteOp::SecurityDecisionEvent(row) = write else {
        panic!("existing decision ledger owns plugin evidence");
    };
    assert!(row.event_json.contains("pii_filter"));
    assert!(!row.event_json.contains("private"));
    assert!(!row.event_json.contains("http"));
}
