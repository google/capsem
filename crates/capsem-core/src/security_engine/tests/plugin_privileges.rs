use super::*;

#[derive(Clone, Copy)]
enum Mutation {
    Add(PolicyActionId),
    Replace,
    Clear,
    Preserve,
}
struct ForgingPlugin {
    stage: SecurityPluginStage,
    mutation: Mutation,
    applied: bool,
}
impl SecurityPlugin for ForgingPlugin {
    fn id(&self) -> &'static str {
        "non_broker"
    }
    fn stage(&self) -> SecurityPluginStage {
        self.stage
    }
    fn apply(
        &self,
        mut event: SecurityEvent,
        _: SecurityPluginConfig,
    ) -> Result<SecurityPluginResult, SecurityActionError> {
        match self.mutation {
            Mutation::Add(action) => event.action_trace.push(action),
            Mutation::Replace => {
                event.action_trace.clear();
                event.action_trace.push(PolicyActionId::CredentialBrokerSubstitute);
            }
            Mutation::Clear => event.action_trace.clear(),
            Mutation::Preserve => {}
        }
        Ok(SecurityPluginResult {
            event,
            applied: self.applied,
        })
    }
}

#[test]
fn plugin_privileges_refuse_non_broker_action_changes_before_emission_in_every_stage() {
    for stage in [
        SecurityPluginStage::Preprocess,
        SecurityPluginStage::Postprocess,
        SecurityPluginStage::Logging,
    ] {
        for applied in [false, true] {
            for mutation in [
                Mutation::Add(PolicyActionId::CredentialBrokerCapture),
                Mutation::Add(PolicyActionId::CredentialBrokerSubstitute),
                Mutation::Replace,
                Mutation::Clear,
            ] {
                let registry = SecurityActionRegistry::new()
                    .with_plugin_policy(BTreeMap::from([(
                        "non_broker".into(),
                        plugin_config(SecurityPluginMode::Rewrite, DetectionLevel::High),
                    )]))
                    .register_plugin(ForgingPlugin {
                        stage,
                        mutation,
                        applied,
                    })
                    .unwrap();
                let emitter = Arc::new(RecordingEmitter::new());
                let engine = SecurityEventEngine::new(registry, emitter.clone());
                let mut event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest);
                event.action_trace.push(PolicyActionId::CredentialBrokerCapture);
                assert!(engine
                    .apply_matching_rules_and_emit(&SecurityRuleSet::new(Vec::new()), event)
                    .is_err());
                assert!(emitter.events.lock().unwrap().is_empty());
            }
        }
    }
}

#[test]
fn plugin_privileges_preserve_existing_broker_actions_during_unrelated_processing() {
    let registry = SecurityActionRegistry::new()
        .with_plugin_policy(BTreeMap::from([(
            "non_broker".into(),
            plugin_config(SecurityPluginMode::Rewrite, DetectionLevel::High),
        )]))
        .register_plugin(ForgingPlugin {
            stage: SecurityPluginStage::Postprocess,
            mutation: Mutation::Preserve,
            applied: true,
        })
        .unwrap();
    let mut event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest);
    event.action_trace = vec![
        PolicyActionId::CredentialBrokerCapture,
        PolicyActionId::CredentialBrokerSubstitute,
    ];
    let engine = SecurityEventEngine::new(registry, Arc::new(RecordingEmitter::new()));
    let result = engine
        .apply_matching_rules_and_emit(&SecurityRuleSet::new(Vec::new()), event)
        .unwrap();
    assert_eq!(
        result.action_trace,
        [
            PolicyActionId::CredentialBrokerCapture,
            PolicyActionId::CredentialBrokerSubstitute
        ]
    );
}

#[test]
fn plugin_privileges_diagnostics_do_not_request_capture_or_substitution() {
    let registry = SecurityActionRegistry::with_builtin_actions().with_plugin_policy(BTreeMap::from([
        (
            "dummy_pre_eicar".into(),
            plugin_config(SecurityPluginMode::Block, DetectionLevel::High),
        ),
        (
            "dummy_post_allow".into(),
            plugin_config(SecurityPluginMode::Allow, DetectionLevel::Low),
        ),
    ]));
    let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http(HttpSecurityEvent {
        body: Some("EICAR diagnostic".into()),
        ..Default::default()
    });
    let engine = SecurityEventEngine::new(registry, Arc::new(RecordingEmitter::new()));
    let result = engine
        .apply_matching_rules_and_emit(&SecurityRuleSet::new(Vec::new()), event)
        .unwrap();
    assert!(
        result.action_trace.is_empty(),
        "diagnostics cannot act as the credential broker"
    );
    assert_eq!(result.decision.effective, SecurityDecisionKind::Block);
    assert_eq!(
        result
            .plugin_executions
            .iter()
            .filter(|execution| execution.applied)
            .count(),
        2
    );
}
