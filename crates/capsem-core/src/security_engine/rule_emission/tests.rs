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
    let write = supplemental_evidence_write(SecurityEventId::new_uuid4(), event.event_type, &event, 100)
        .unwrap()
        .unwrap();
    let WriteOp::SecurityDecisionEvent(row) = write else {
        panic!("existing decision ledger owns plugin evidence");
    };
    assert!(row.event_json.contains("pii_filter"));
    assert!(!row.event_json.contains("private"));
    assert!(!row.event_json.contains("http"));
}

#[test]
fn observation_only_evidence_records_a_snapshot_without_a_decision_transition() {
    let mut event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest);
    let id = SecurityEventId::new_uuid4();
    assert!(supplemental_evidence_write(id.clone(), event.event_type, &event, 100)
        .unwrap()
        .is_none());
    event.request_decision(SecurityDecisionKind::Block);
    event.record_detection(SecurityDetectionEvent {
        source: SecurityDetectionSource::Rule,
        detection_level: DetectionLevel::High,
        rule_id: Some("environment.detection.probe".into()),
        plugin_id: None,
        action: None,
        plugin_mode: None,
        reason: None,
    });
    let WriteOp::SecurityDecisionEvent(row) = supplemental_evidence_write(id, event.event_type, &event, 100)
        .unwrap()
        .unwrap()
    else {
        panic!("existing decision ledger owns the observation snapshot");
    };
    assert_eq!(row.actor, "security.observations");
    assert_eq!(row.stage, LoggedSecurityDecisionStage::Rule);
    assert_eq!(row.previous_decision, LoggedSecurityDecision::Block);
    assert_eq!(row.requested_decision, row.previous_decision);
    assert_eq!(row.effective_decision, row.previous_decision);
    assert!(row.plugin_id.is_none());
    assert!(row.rule_id.is_none());
    assert!(row.event_json.contains("environment.detection.probe"));
}
