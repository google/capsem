use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityEmitError {
    message: String,
}

impl SecurityEmitError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for SecurityEmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for SecurityEmitError {}

/// Single auditable event emission boundary.
pub trait SecurityEventEmitter: Send + Sync {
    fn emit(&self, event: SecurityEvent) -> Result<(), SecurityEmitError>;
}

/// Security-event execution boundary for matched rule actions.
///
/// Runtime/parser paths hand this engine a canonical `SecurityEvent` plus the
/// matched action-bearing rules. The engine applies actions in deterministic
/// order, then emits exactly the final post-action event.
pub struct SecurityEventEngine<E: SecurityEventEmitter> {
    action_registry: SecurityActionRegistry,
    emitter: Arc<E>,
}

impl<E: SecurityEventEmitter> SecurityEventEngine<E> {
    pub fn new(action_registry: SecurityActionRegistry, emitter: Arc<E>) -> Self {
        Self {
            action_registry,
            emitter,
        }
    }

    pub fn with_builtin_actions(emitter: Arc<E>) -> Self {
        Self::new(SecurityActionRegistry::with_builtin_actions(), emitter)
    }

    pub fn apply_matching_rules_and_emit(
        &self,
        rules: &SecurityRuleSet,
        mut event: SecurityEvent,
    ) -> Result<SecurityEvent, SecurityActionError> {
        event.validate_network(event.event_type)?;
        event = self
            .action_registry
            .apply_security_plugins(SecurityPluginStage::Preprocess, event)?;

        let evaluation = rules.evaluate(&event).map_err(SecurityActionError::new)?;
        for rule in evaluation.matched_rules() {
            record_rule_detection(&mut event, rule);
        }
        event.request_decision(requested_boundary_decision(
            selected_enforcement_rule(&evaluation),
            event.event_type,
        ));
        event = self
            .action_registry
            .apply_security_plugins(SecurityPluginStage::Postprocess, event)?;
        event = self
            .action_registry
            .apply_security_plugins(SecurityPluginStage::Logging, event)?;
        self.emitter
            .emit(event.clone())
            .map_err(|error| SecurityActionError::new(error.to_string()))?;
        Ok(event)
    }
}
