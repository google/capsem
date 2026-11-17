use crate::net::policy_config::SecurityPluginConfig;
use crate::security_engine::{
    SecurityActionError, SecurityDecisionKind, SecurityEvent, SecurityPlugin, SecurityPluginResult, SecurityPluginStage,
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
        Ok(SecurityPluginResult::applied(event))
    }
}
