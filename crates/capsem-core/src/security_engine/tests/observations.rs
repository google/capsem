use super::*;

fn rules(action: Option<&str>) -> SecurityRuleSet {
    let access = action
        .map(|action| {
            SecurityRuleProfile::parse_toml(&format!(
                "[corp.rules.guard]\nname='guard'\naction='{action}'\nmatch='http.host == \"observed.example\"'"
            ))
            .unwrap()
            .compile(SecurityRuleSource::Corp)
            .unwrap()
        })
        .unwrap_or_default();
    let observation = capsem_config::CompiledEnvironmentDetectionRule::new(
        "probe",
        &capsem_config::environment::EnvironmentDetectionRule {
            name: "Probe".into(),
            condition: "http.host == \"observed.example\"".into(),
            level: DetectionLevel::High,
            enabled: true,
        },
    )
    .unwrap();
    SecurityRuleSet::with_observations(access, vec![observation])
}

fn event() -> SecurityEvent {
    SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http(HttpSecurityEvent {
        host: Some("observed.example".into()),
        body: Some("private body must not be copied into evidence-only rows".into()),
        ..Default::default()
    })
}

fn assert_observation(event: &SecurityEvent) {
    let detections: Vec<_> = event
        .detections
        .iter()
        .filter(|d| d.rule_id.as_deref() == Some("environment.detection.probe"))
        .collect();
    assert_eq!(detections.len(), 1);
    assert_eq!(detections[0].detection_level, DetectionLevel::High);
    assert_eq!(detections[0].source, SecurityDetectionSource::Rule);
    assert_eq!(detections[0].action, None);
    assert_eq!(detections[0].plugin_id, None);
}

#[test]
fn environment_observations_reach_engine_and_boundaries_without_decisions() {
    for (action, expected) in [
        (None, SecurityDecisionKind::Allow),
        (Some("allow"), SecurityDecisionKind::Allow),
        (Some("ask"), SecurityDecisionKind::Ask),
        (Some("block"), SecurityDecisionKind::Block),
    ] {
        let rules = rules(action);
        let emitter = Arc::new(RecordingEmitter::new());
        let engine = SecurityEventEngine::new(SecurityActionRegistry::new(), emitter.clone());
        let evaluated = engine.apply_matching_rules_and_emit(&rules, event()).unwrap();
        assert_observation(&evaluated);
        assert_eq!(evaluated.decision.effective, expected);
        assert_eq!(emitter.events.lock().unwrap().len(), 1);
        let boundary =
            evaluate_security_boundary_with_registry(&rules, SecurityActionRegistry::new(), event()).unwrap();
        assert_observation(&boundary.event);
        assert_eq!(boundary.event.decision.effective, expected);
        let repeated = engine.apply_matching_rules_and_emit(&rules, evaluated).unwrap();
        assert_observation(&repeated);
    }
}

#[tokio::test]
async fn environment_observations_only_survive_async_and_blocking_ledger_emission() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&path, 16).unwrap();
    let rules = rules(None);
    let engine = SecurityEventEngine::new(SecurityActionRegistry::new(), Arc::new(RecordingEmitter::new()));
    let evaluated = engine.apply_matching_rules_and_emit(&rules, event()).unwrap();
    let mut ids = Vec::new();
    for blocking in [false, true] {
        let id = SecurityEventId::new_uuid4();
        let result = if blocking {
            emit_matching_security_rules_with_decision_blocking(
                &writer,
                id.clone(),
                evaluated.event_type,
                &rules,
                &evaluated,
                100,
            )
            .unwrap()
        } else {
            emit_matching_security_rules_with_decision(
                &writer,
                id.clone(),
                evaluated.event_type,
                &rules,
                &evaluated,
                100,
            )
            .await
            .unwrap()
        };
        assert_observation(&result.event);
        assert_eq!(result.emitted, 0);
        assert!(result.rule_events.is_empty());
        assert_eq!(result.enforcement.action, SecurityEnforcementAction::Allow);
        assert!(result.enforcement.ask_id.is_none());
        ids.push(id);
    }
    writer.flush_checked().await.unwrap();
    writer.shutdown_blocking();
    for id in ids {
        let payload: serde_json::Value = serde_json::from_str(
            &super::decision_ledger::archived_payload_of(&path, "security_decision_events", id.as_str()).await,
        )
        .unwrap();
        assert_eq!(payload["detections"].as_array().unwrap().len(), 1);
        assert_eq!(payload["detections"][0]["rule_id"], "environment.detection.probe");
        assert!(payload["detections"][0]["action"].is_null());
        assert!(payload.get("http").is_none());
        assert!(!payload.to_string().contains("private body"));
    }
}
