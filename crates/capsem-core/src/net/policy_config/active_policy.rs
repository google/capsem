//! The policy one session enforces.
//!
//! Built-in defaults, the user's settings.toml and the corp config are merged
//! into one file per session, `vm/active_policy.toml`. capsem-process and
//! capsem-mcp-builtin read only that file, so what a VM enforces is exactly
//! what the service published, and nothing re-reads the user's files behind
//! its back.
//!
//! The file has no identity of its own: it is a pure function of the three
//! inputs. A file in any other shape, such as one carrying an id and revision,
//! is refused, not adapted.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::builder::{merge_plugin_policy, network_config_from_policy_and_dns, MergedPolicies};
use super::provider_profile::{ModelEndpointRegistry, ProviderRuleProfile};
use super::security_rule_profile::{SecurityPluginConfig, SecurityRuleProfile, SecurityRuleSet};
use super::types::{NetworkConfig, SettingsFile};
use crate::mcp::policy::McpConfig;
use capsem_config::validate_policy_target;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivePolicyFile {
    /// The user's rules from settings.toml (inline and referenced files).
    pub user_rules: SecurityRuleProfile,
    /// The corp config's rules; a rule here replaces the user's of the same id.
    pub corp_rules: SecurityRuleProfile,
    /// Built-in plugin modes, then the user's, then corp's.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub plugins: BTreeMap<String, SecurityPluginConfig>,
    pub network: NetworkConfig,
    /// The user's MCP servers with corp's laid over them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp: Option<McpConfig>,
}

/// Digest of an active policy's serialized bytes, as `blake3:<hex>`.
///
/// The service writes a session's active policy and capsem-process loads it
/// on reload. Both sides name what they handled by the digest of the exact
/// bytes, so an acknowledgement proves which policy a VM applied rather than
/// merely that it applied one.
pub fn active_policy_digest(bytes: &[u8]) -> String {
    format!("blake3:{}", blake3::hash(bytes).to_hex())
}

impl ActivePolicyFile {
    /// Merge the user's settings.toml with the corp config. Fails when either
    /// side's rules do not compile: an empty rule set would allow everything.
    pub fn from_settings_and_corp(settings: &SettingsFile, corp: &SettingsFile) -> Result<Self, String> {
        settings.validate_metadata_contract()?;
        corp.validate_metadata_contract()?;
        let user_rules = SecurityRuleProfile {
            default: settings.default.clone(),
            profiles: settings.profiles.clone(),
            ai: settings.ai.clone(),
            ..SecurityRuleProfile::default()
        };
        let corp_rules = SecurityRuleProfile {
            default: corp.default.clone(),
            profiles: corp.profiles.clone(),
            corp: corp.corp.clone(),
            ai: corp.ai.clone(),
            plugins: BTreeMap::new(),
        };
        let merged = MergedPolicies::from_files(settings, corp)?;
        let mut network = network_config_from_policy_and_dns(&merged.network, corp.network.dns.clone());
        network.upstream_overrides = corp.network.upstream_overrides.clone();

        let active = Self {
            user_rules,
            corp_rules,
            plugins: merge_plugin_policy(settings, corp),
            network,
            mcp: McpConfig::merged(settings.mcp.as_ref(), corp.mcp.as_ref()),
        };
        active.validate()?;
        Ok(active)
    }

    pub fn validate(&self) -> Result<(), String> {
        self.user_rules.validate()?;
        self.corp_rules.validate()?;
        for plugin_id in self.plugins.keys() {
            validate_policy_target("plugin id", plugin_id)?;
        }
        self.network.validate()?;
        if let Some(mcp) = &self.mcp {
            mcp.validate("active_policy")?;
        }
        Ok(())
    }

    /// The (user, corp) settings pair `MergedPolicies` builds a session from.
    pub fn merged_policy_inputs(&self) -> (SettingsFile, SettingsFile) {
        let user = SettingsFile {
            default: self.user_rules.default.clone(),
            profiles: self.user_rules.profiles.clone(),
            ai: self.user_rules.ai.clone(),
            ..SettingsFile::default()
        };
        let corp = SettingsFile {
            default: self.corp_rules.default.clone(),
            profiles: self.corp_rules.profiles.clone(),
            corp: self.corp_rules.corp.clone(),
            ai: self.corp_rules.ai.clone(),
            network: self.network.clone(),
            ..SettingsFile::default()
        };
        (user, corp)
    }

    pub fn compile_security_rule_set(&self) -> Result<SecurityRuleSet, String> {
        self.validate()?;
        let (user, corp) = self.merged_policy_inputs();
        Ok(MergedPolicies::from_files(&user, &corp)?.security_rules)
    }

    pub fn model_endpoint_registry(&self) -> Result<ModelEndpointRegistry, String> {
        self.validate()?;
        let provider_profile = ProviderRuleProfile::merge_defaults_user_and_corp(
            &ProviderRuleProfile {
                ai: self.user_rules.ai.clone(),
            },
            &ProviderRuleProfile {
                ai: self.corp_rules.ai.clone(),
            },
        )?;
        provider_profile.endpoint_registry()
    }
}

#[cfg(test)]
mod tests;
