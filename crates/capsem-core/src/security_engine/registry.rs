//! One plugin execution rail and its credential-action privilege boundary.
use super::*;

#[derive(Default, Clone)]
pub struct SecurityActionRegistry {
    pub(super) plugins: Arc<BTreeMap<String, Arc<dyn SecurityPlugin>>>,
    pub(super) plugin_policy: Arc<BTreeMap<String, SecurityPluginConfig>>,
}

impl SecurityActionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_builtin_actions() -> Self {
        builtin_actions::registry().clone()
    }

    pub fn with_plugin_policy(mut self, plugin_policy: impl Into<Arc<BTreeMap<String, SecurityPluginConfig>>>) -> Self {
        self.plugin_policy = plugin_policy.into();
        self
    }

    pub(super) fn with_proxy_credentials(
        mut self,
        credentials: Arc<dyn crate::net::proxy_engine::ProxyCredentials>,
    ) -> Self {
        Arc::make_mut(&mut self.plugins).insert(
            "credential_broker".to_string(),
            Arc::new(CredentialBrokerPlugin::new(credentials)),
        );
        self
    }

    pub fn register_plugin(mut self, plugin: impl SecurityPlugin + 'static) -> Result<Self, SecurityActionError> {
        let id = plugin.id();
        if self.plugins.contains_key(id) {
            return Err(SecurityActionError::new(format!(
                "security plugin '{id}' registered twice"
            )));
        }
        Arc::make_mut(&mut self.plugins).insert(id.to_string(), Arc::new(plugin));
        Ok(self)
    }

    pub fn apply_security_plugins(
        &self,
        stage: SecurityPluginStage,
        mut event: SecurityEvent,
    ) -> Result<SecurityEvent, SecurityActionError> {
        for (plugin_id, config) in self.plugin_policy.iter() {
            if config.mode != SecurityPluginMode::Disable && !self.plugins.contains_key(plugin_id) {
                return Err(SecurityActionError::new(format!(
                    "security plugin '{plugin_id}' is not registered"
                )));
            }
        }
        for (plugin_id, plugin) in self.plugins.iter() {
            if plugin.stage() != stage {
                continue;
            }
            let Some(plugin_config) = self.plugin_policy.get(plugin_id).copied() else {
                continue;
            };
            if plugin_config.mode == SecurityPluginMode::Disable {
                continue;
            }
            let started = std::time::Instant::now();
            let protected_actions = (plugin_id != "credential_broker").then(|| credential_action_counts(&event));
            let result = match plugin.apply(event, plugin_config) {
                Ok(result) => result,
                Err(error) => {
                    record_plugin_metrics(plugin_id, stage, plugin_config.mode, false, "error", started);
                    return Err(error);
                }
            };
            let duration_us = started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64;
            if protected_actions.is_some_and(|before| credential_action_counts(&result.event) != before) {
                record_plugin_metrics(plugin_id, stage, plugin_config.mode, false, "error", started);
                return Err(SecurityActionError::new(
                    "only credential_broker may alter credential action requests",
                ));
            }
            record_plugin_metrics(plugin_id, stage, plugin_config.mode, result.applied, "ok", started);
            event = result.event;
            event.record_plugin_execution(SecurityPluginExecution {
                plugin_id: plugin_id.clone(),
                stage,
                applied: result.applied,
                duration_us,
            });
            if !result.applied {
                continue;
            }
            record_plugin_detection(&mut event, plugin_id, plugin_config);
            if let Some(requested) = plugin_mode_decision(plugin_config.mode) {
                event.request_decision(requested);
            }
        }
        Ok(event)
    }
}

fn credential_action_counts(event: &SecurityEvent) -> [usize; 2] {
    let mut counts = [0; 2];
    for action in &event.action_trace {
        match action {
            PolicyActionId::CredentialBrokerCapture => counts[0] += 1,
            PolicyActionId::CredentialBrokerSubstitute => counts[1] += 1,
        }
    }
    counts
}
