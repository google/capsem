//! Separate validation plugin: UTF-8 validity is not ASCII-only filtering.
use crate::environment::CompiledPluginConfiguration;
use crate::net::policy_config::SecurityPluginMode;
use crate::security_engine::{
    SecurityActionError, SecurityDecisionKind, SecurityEvent, SecurityPlugin, SecurityPluginResult, SecurityPluginStage,
};

pub(in crate::security_engine) struct Utf8GuardPlugin;

impl SecurityPlugin for Utf8GuardPlugin {
    fn id(&self) -> &'static str {
        "utf8_guard"
    }
    fn stage(&self) -> SecurityPluginStage {
        SecurityPluginStage::Preprocess
    }
    fn apply(
        &self,
        mut event: SecurityEvent,
        config: &CompiledPluginConfiguration,
    ) -> Result<SecurityPluginResult, SecurityActionError> {
        if event.http_body_utf8_valid != Some(false) {
            return Ok(SecurityPluginResult::skipped(event));
        }
        // There is no lossy rewrite mode. A rewrite request fails closed rather
        // than replacing bytes and silently changing an application's protocol.
        if config.mode == SecurityPluginMode::Rewrite {
            event.request_decision(SecurityDecisionKind::Block);
        }
        Ok(SecurityPluginResult::applied(event))
    }
}
