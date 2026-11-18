//! Configuration admission for the single plugin executor. The legacy input
//! adapter is confined here while settings/publication migrate to environments;
//! plugins themselves receive only borrowed, typed compiled configurations.
use super::*;
use crate::environment::{CompiledEnvironmentPlugins, EnvironmentPluginSource, PluginDirection};
use capsem_config::plugins::PluginId;
use std::borrow::Cow;

/// Immutable runtime capability. Environment configurations retain their
/// typed directional options; settings snapshots remain an explicit migration
/// input to the same executor, never an option-flattening adapter.
#[derive(Clone)]
pub enum SecurityPluginPolicy {
    Settings(Arc<BTreeMap<String, SecurityPluginConfig>>),
    Environment(Arc<CompiledEnvironmentPlugins>),
}
impl Default for SecurityPluginPolicy {
    fn default() -> Self {
        Self::Settings(Arc::default())
    }
}
impl SecurityPluginPolicy {
    /// Number of resolved plugin identities, including explicitly disabled ones.
    pub fn configured_plugin_count(&self) -> usize {
        match self {
            Self::Settings(policy) => policy.len(),
            Self::Environment(_) => PluginId::ALL.len(),
        }
    }

    /// Snapshot identity, not configuration equality or publication authority.
    /// Cloning retains identity; replacing an immutable policy does not.
    pub fn same_snapshot(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Settings(left), Self::Settings(right)) => Arc::ptr_eq(left, right),
            (Self::Environment(left), Self::Environment(right)) => Arc::ptr_eq(left, right),
            _ => false,
        }
    }

    pub fn configuration(&self, id: &str, direction: PluginDirection) -> Option<Cow<'_, CompiledPluginConfiguration>> {
        match self {
            Self::Settings(policy) => policy
                .get(id)
                .map(|config| Cow::Owned(legacy_configuration(id, *config, direction))),
            Self::Environment(policy) => PluginId::ALL
                .iter()
                .find(|plugin| plugin.as_str() == id)
                .map(|id| Cow::Borrowed(policy.configuration(*id, direction))),
        }
    }
    pub(super) fn validate_registration(
        &self,
        plugins: &BTreeMap<String, Arc<dyn SecurityPlugin>>,
        direction: PluginDirection,
    ) -> Result<(), SecurityActionError> {
        let check = |id: &str| {
            if self
                .configuration(id, direction)
                .is_some_and(|config| config.mode != SecurityPluginMode::Disable)
                && !plugins.contains_key(id)
            {
                return Err(SecurityActionError::new(format!(
                    "security plugin '{id}' is not registered"
                )));
            }
            Ok(())
        };
        match self {
            Self::Settings(policy) => {
                for id in policy.keys() {
                    check(id)?;
                }
            }
            Self::Environment(_) => {
                for id in PluginId::ALL {
                    check(id.as_str())?;
                }
            }
        }
        Ok(())
    }
}

pub(super) fn direction(event: &SecurityEvent) -> PluginDirection {
    if event.http.as_ref().is_some_and(|http| http.status.is_some()) {
        PluginDirection::Inbound
    } else {
        PluginDirection::Outbound
    }
}

pub(super) fn legacy_configuration(
    id: &str,
    config: SecurityPluginConfig,
    direction: PluginDirection,
) -> CompiledPluginConfiguration {
    let options = match id {
        "pii_filter" => CompiledPluginOptions::PiiFilter(capsem_config::plugins::PiiFilterOptions {
            email: config.content.email,
            phone: config.content.phone,
            card: config.content.card,
            ssn: config.content.ssn,
        }),
        "secret_exfiltration" => CompiledPluginOptions::SecretExfiltration,
        "utf8_guard" => CompiledPluginOptions::Utf8Guard,
        "credential_broker" => CompiledPluginOptions::CredentialBroker { capture_enabled: true },
        "log_sanitizer" => CompiledPluginOptions::LogSanitizer,
        "dummy_pre_eicar" => CompiledPluginOptions::DummyPreEicar,
        "dummy_post_allow" => CompiledPluginOptions::DummyPostAllow,
        _ => CompiledPluginOptions::NoOptions,
    };
    let mode = if matches!(id, "pii_filter" | "secret_exfiltration" | "utf8_guard")
        && !config.content.direction.includes(direction == PluginDirection::Inbound)
    {
        SecurityPluginMode::Disable
    } else {
        config.mode
    };
    CompiledPluginConfiguration {
        mode,
        detection_level: config.detection_level,
        source: EnvironmentPluginSource::Environment,
        options,
    }
}

impl From<Arc<BTreeMap<String, SecurityPluginConfig>>> for SecurityPluginPolicy {
    fn from(policy: Arc<BTreeMap<String, SecurityPluginConfig>>) -> Self {
        Self::Settings(policy)
    }
}
impl From<BTreeMap<String, SecurityPluginConfig>> for SecurityPluginPolicy {
    fn from(policy: BTreeMap<String, SecurityPluginConfig>) -> Self {
        Self::Settings(Arc::new(policy))
    }
}
impl From<Arc<CompiledEnvironmentPlugins>> for SecurityPluginPolicy {
    fn from(policy: Arc<CompiledEnvironmentPlugins>) -> Self {
        Self::Environment(policy)
    }
}
impl std::fmt::Debug for SecurityPluginPolicy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Settings(policy) => formatter.debug_tuple("Settings").field(policy).finish(),
            Self::Environment(_) => formatter.write_str("Environment(..)"),
        }
    }
}

impl From<&SecurityPluginPolicy> for SecurityPluginPolicy {
    fn from(policy: &SecurityPluginPolicy) -> Self {
        policy.clone()
    }
}
impl From<&BTreeMap<String, SecurityPluginConfig>> for SecurityPluginPolicy {
    fn from(policy: &BTreeMap<String, SecurityPluginConfig>) -> Self {
        Self::Settings(Arc::new(policy.clone()))
    }
}
