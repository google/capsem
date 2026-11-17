//! One plugin execution rail and its credential-action privilege boundary.
use super::*;

#[derive(Default, Clone)]
pub struct SecurityActionRegistry {
    pub(super) plugins: Arc<BTreeMap<String, Arc<dyn SecurityPlugin>>>,
    pub(super) policy: runtime_policy::RuntimePluginPolicy,
}

impl SecurityActionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_builtin_actions() -> Self {
        builtin_actions::registry().clone()
    }

    pub fn with_plugin_policy(mut self, plugin_policy: impl Into<Arc<BTreeMap<String, SecurityPluginConfig>>>) -> Self {
        self.policy = runtime_policy::RuntimePluginPolicy::Settings(plugin_policy.into());
        self
    }

    pub fn with_environment_plugins(mut self, plugins: Arc<crate::environment::CompiledEnvironmentPlugins>) -> Self {
        self.policy = runtime_policy::RuntimePluginPolicy::Environment(plugins);
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
        let direction = runtime_policy::direction(&event);
        self.policy.validate_registration(&self.plugins, direction)?;
        for (plugin_id, plugin) in self.plugins.iter() {
            if plugin.stage() != stage {
                continue;
            }
            let Some(plugin_config) = self.policy.configuration(plugin_id, direction) else {
                continue;
            };
            if plugin_config.mode == SecurityPluginMode::Disable {
                continue;
            }
            let started = std::time::Instant::now();
            let protected_actions = (plugin_id != "credential_broker").then(|| credential_action_counts(&event));
            let result = match plugin.apply(event, &plugin_config) {
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
            record_plugin_detection(&mut event, plugin_id, &plugin_config);
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

fn record_plugin_metrics(
    plugin_id: &str,
    stage: SecurityPluginStage,
    mode: SecurityPluginMode,
    applied: bool,
    status: &'static str,
    started: Instant,
) {
    let stage = stage.as_str();
    let mode = mode.as_str();
    let applied = if applied { "true" } else { "false" };
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    ::metrics::counter!(
        SECURITY_PLUGIN_EXECUTION_TOTAL,
        "plugin_id" => plugin_id.to_string(),
        "stage" => stage,
        "mode" => mode,
        "applied" => applied,
        "status" => status,
    )
    .increment(1);
    ::metrics::histogram!(
        SECURITY_PLUGIN_EXECUTION_DURATION_MS,
        "plugin_id" => plugin_id.to_string(),
        "stage" => stage,
        "mode" => mode,
        "applied" => applied,
        "status" => status,
    )
    .record(elapsed_ms);
    tracing::trace!(
        target: "capsem.security_plugin",
        plugin_id,
        stage,
        mode,
        applied,
        status,
        elapsed_ms,
        "security plugin execution"
    );
}

fn record_plugin_detection(event: &mut SecurityEvent, plugin_id: &str, config: &CompiledPluginConfiguration) {
    let Some(detection_level) = config.active_detection_level() else {
        return;
    };
    event.record_detection(SecurityDetectionEvent {
        source: SecurityDetectionSource::Plugin,
        detection_level,
        rule_id: None,
        plugin_id: Some(plugin_id.to_string()),
        action: None,
        plugin_mode: Some(config.mode),
        reason: None,
    });
}

fn plugin_mode_decision(mode: SecurityPluginMode) -> Option<SecurityDecisionKind> {
    match mode {
        SecurityPluginMode::Disable => None,
        SecurityPluginMode::Allow | SecurityPluginMode::Rewrite => Some(SecurityDecisionKind::Allow),
        SecurityPluginMode::Ask => Some(SecurityDecisionKind::Ask),
        SecurityPluginMode::Block => Some(SecurityDecisionKind::Block),
    }
}
