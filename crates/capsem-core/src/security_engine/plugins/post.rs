use crate::net::policy_config::{PolicyActionId, SecurityPluginConfig};
use crate::security_engine::{
    SecurityActionError, SecurityDecisionKind, SecurityEvent, SecurityPlugin, SecurityPluginResult,
    SecurityPluginStage,
};

pub(in crate::security_engine) struct DummyPostAllowPlugin;

impl SecurityPlugin for DummyPostAllowPlugin {
    fn id(&self) -> &'static str {
        "dummy_post_allow"
    }

    fn stage(&self) -> SecurityPluginStage {
        SecurityPluginStage::Postprocess
    }

    fn apply(
        &self,
        mut event: SecurityEvent,
        _config: SecurityPluginConfig,
    ) -> Result<SecurityPluginResult, SecurityActionError> {
        event.request_decision(SecurityDecisionKind::Allow);
        event
            .action_trace
            .push(PolicyActionId::CredentialBrokerSubstitute);
        Ok(SecurityPluginResult::applied(event))
    }
}
