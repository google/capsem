//! Typed plugin compilation. No settings lookup, parsing or mutable policy is
//! involved in runtime selection. Callers supply the acknowledged boot options
//! and separately loaded current typed corporate overrides.
use capsem_config::{plugins::*, DetectionLevel, SecurityPluginMode};
use std::collections::BTreeMap;

pub use capsem_config::plugins::{EnvironmentPluginSource, PluginDirection};

/// Each plugin owns its option contract; unrelated plugins never receive a
/// PII-shaped configuration bag. More complex plugins can compile their own
/// immutable payload here instead of parsing data during event processing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompiledPluginOptions {
    /// Registered extensions without builtin-specific options. This is an
    /// internal invocation value, never a persisted environment option bag.
    NoOptions,
    PiiFilter(PiiFilterOptions),
    SecretExfiltration,
    Utf8Guard,
    CredentialBroker {
        capture_enabled: bool,
    },
    LogSanitizer,
    DummyPreEicar,
    DummyPostAllow,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledPluginConfiguration {
    pub mode: SecurityPluginMode,
    pub detection_level: DetectionLevel,
    pub source: EnvironmentPluginSource,
    pub options: CompiledPluginOptions,
}
impl CompiledPluginConfiguration {
    pub const fn locked(&self) -> bool {
        matches!(self.source, EnvironmentPluginSource::Corporate)
    }
    pub const fn active_detection_level(&self) -> Option<DetectionLevel> {
        if matches!(self.mode, SecurityPluginMode::Disable) {
            None
        } else {
            Some(self.detection_level)
        }
    }
}

pub struct CompiledEnvironmentPlugins {
    inbound: BTreeMap<PluginId, CompiledPluginConfiguration>,
    outbound: BTreeMap<PluginId, CompiledPluginConfiguration>,
}

fn selected<T: Clone + Default>(environment: &Option<T>, corporate: &Option<T>) -> (T, EnvironmentPluginSource) {
    if let Some(config) = corporate {
        (config.clone(), EnvironmentPluginSource::Corporate)
    } else if let Some(config) = environment {
        (config.clone(), EnvironmentPluginSource::Environment)
    } else {
        (T::default(), EnvironmentPluginSource::Builtin)
    }
}
fn compiled<O: PluginOptions>(
    config: &PluginConfiguration<O>,
    source: EnvironmentPluginSource,
    options: CompiledPluginOptions,
) -> CompiledPluginConfiguration {
    CompiledPluginConfiguration {
        mode: config.mode,
        detection_level: config.detection_level,
        source,
        options,
    }
}

impl CompiledEnvironmentPlugins {
    pub fn with_corporate(environment: &EnvironmentPlugins, corporate: &capsem_config::SettingsFile) -> Result<Self, String> {
        corporate.validate_metadata_contract()?;
        if !corporate.plugins.is_empty() {
            return Err("environment policy requires typed corporate environment_plugins, not legacy plugin modes".into());
        }
        let defaults = EnvironmentPlugins::default();
        Self::resolve(environment, corporate.environment_plugins.as_ref().unwrap_or(&defaults))
    }

    /// Resolve whole plugin entries, never merge corporate fields into an
    /// environment's entry. The source/lock metadata describes this resolution;
    /// it is not caller authority or a policy publication acknowledgement.
    pub fn resolve(environment: &EnvironmentPlugins, corporate: &EnvironmentPlugins) -> Result<Self, String> {
        environment.validate().map_err(String::from)?;
        corporate.validate().map_err(String::from)?;
        let mut result = Self {
            inbound: BTreeMap::new(),
            outbound: BTreeMap::new(),
        };

        let (pii, source) = selected(&environment.pii_filter, &corporate.pii_filter);
        result.inbound.insert(
            PluginId::PiiFilter,
            compiled(
                &pii.inbound,
                source,
                CompiledPluginOptions::PiiFilter(pii.inbound.options.clone()),
            ),
        );
        result.outbound.insert(
            PluginId::PiiFilter,
            compiled(
                &pii.outbound,
                source,
                CompiledPluginOptions::PiiFilter(pii.outbound.options.clone()),
            ),
        );

        let (utf8, source) = selected(&environment.utf8_guard, &corporate.utf8_guard);
        let mut inbound = compiled(&utf8, source, CompiledPluginOptions::Utf8Guard);
        let mut outbound = inbound.clone();
        if !utf8.options.inbound {
            inbound.mode = SecurityPluginMode::Disable;
        }
        if !utf8.options.outbound {
            outbound.mode = SecurityPluginMode::Disable;
        }
        result.inbound.insert(PluginId::Utf8Guard, inbound);
        result.outbound.insert(PluginId::Utf8Guard, outbound);

        let (broker, source) = selected(&environment.credential_broker, &corporate.credential_broker);
        result.insert_both(
            PluginId::CredentialBroker,
            compiled(
                &broker,
                source,
                CompiledPluginOptions::CredentialBroker {
                    capture_enabled: broker.options.capture_enabled,
                },
            ),
        );
        let (sanitizer, source) = selected(&environment.log_sanitizer, &corporate.log_sanitizer);
        result.insert_both(
            PluginId::LogSanitizer,
            compiled(&sanitizer, source, CompiledPluginOptions::LogSanitizer),
        );
        let (exfiltration, source) = selected(&environment.secret_exfiltration, &corporate.secret_exfiltration);
        result.insert_both(
            PluginId::SecretExfiltration,
            compiled(&exfiltration, source, CompiledPluginOptions::SecretExfiltration),
        );
        let (pre, source) = selected(&environment.dummy_pre_eicar, &corporate.dummy_pre_eicar);
        result.insert_both(
            PluginId::DummyPreEicar,
            compiled(&pre, source, CompiledPluginOptions::DummyPreEicar),
        );
        let (post, source) = selected(&environment.dummy_post_allow, &corporate.dummy_post_allow);
        result.insert_both(
            PluginId::DummyPostAllow,
            compiled(&post, source, CompiledPluginOptions::DummyPostAllow),
        );
        Ok(result)
    }

    fn insert_both(&mut self, id: PluginId, config: CompiledPluginConfiguration) {
        self.inbound.insert(id, config.clone());
        self.outbound.insert(id, config);
    }

    /// Direction is relative to the sandbox. This selects prepared data and
    /// does not imply that a plugin supports every protocol in that direction.
    pub fn configuration(&self, id: PluginId, direction: PluginDirection) -> &CompiledPluginConfiguration {
        let configs = match direction {
            PluginDirection::Inbound => &self.inbound,
            PluginDirection::Outbound => &self.outbound,
        };
        configs
            .get(&id)
            .expect("all builtin plugin configurations are compiled")
    }
}

#[cfg(test)]
mod tests;

impl super::EnvironmentPolicyInputs<'_> {
    pub fn compile_plugins(&self) -> Result<CompiledEnvironmentPlugins, String> {
        CompiledEnvironmentPlugins::with_corporate(self.plugins(), self.corporate())
    }
}
