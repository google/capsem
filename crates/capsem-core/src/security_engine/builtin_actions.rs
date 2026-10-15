//! The built-in security plugin set, built once.
//!
//! `SecurityActionRegistry::with_builtin_actions()` used to construct the
//! four built-in plugins and a fresh map on every evaluated event: two or
//! three times per proxied request, once per DNS query, three times per MCP
//! call. The plugins are stateless, so one registry is built on first use
//! and shared through an `Arc` for each evaluation; the
//! per-evaluation plugin policy is still applied on the clone.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use crate::net::policy_config::SecurityPluginConfig;

use super::{
    CredentialBrokerPlugin, DummyPostAllowPlugin, DummyPreEicarPlugin, LogSanitizerPlugin, SecurityActionRegistry,
};

pub(super) fn registry() -> &'static SecurityActionRegistry {
    static BUILTIN: OnceLock<SecurityActionRegistry> = OnceLock::new();
    BUILTIN.get_or_init(|| {
        SecurityActionRegistry::new()
            .register_plugin(CredentialBrokerPlugin::new(std::sync::Arc::new(
                crate::net::proxy_engine::LocalProxyCredentials,
            )))
            .and_then(|registry| registry.register_plugin(DummyPreEicarPlugin))
            .and_then(|registry| registry.register_plugin(DummyPostAllowPlugin))
            .and_then(|registry| registry.register_plugin(LogSanitizerPlugin))
            .and_then(|registry| registry.register_plugin(super::plugins::PiiFilterPlugin))
            .and_then(|registry| registry.register_plugin(super::plugins::SecretExfiltrationPlugin))
            .and_then(|registry| registry.register_plugin(super::plugins::Utf8GuardPlugin))
            .expect("built-in security plugin ids are unique")
    })
}

impl SecurityActionRegistry {
    pub fn any_plugin_requires_buffered_model_response(
        &self,
        plugin_policy: &BTreeMap<String, SecurityPluginConfig>,
    ) -> bool {
        self.plugins.iter().any(|(plugin_id, plugin)| {
            plugin_policy
                .get(plugin_id)
                .copied()
                .is_some_and(|config| plugin.requires_buffered_model_response(config))
        })
    }
}

#[cfg(test)]
mod tests;
