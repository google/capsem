use anyhow::{Context, Result};
use capsem_core::mcp::types::McpServerDef;
use capsem_core::net::policy::NetworkPolicy;
use capsem_core::net::policy_config::{
    ModelEndpointRegistry, Profile, ProviderRuleProfile, SecurityPluginConfig, SecurityRuleSet,
    SecurityRuleSource,
};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub(crate) struct RuntimeProfileSource {
    profile_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub(crate) struct RuntimeProfileConfig {
    pub(crate) profile_id: String,
    pub(crate) profile_dir: PathBuf,
    pub(crate) config_root: PathBuf,
    pub(crate) network: NetworkPolicy,
    pub(crate) security_rules: SecurityRuleSet,
    pub(crate) plugins: BTreeMap<String, SecurityPluginConfig>,
    pub(crate) model_endpoints: ModelEndpointRegistry,
    pub(crate) mcp: capsem_core::mcp::policy::McpUserConfig,
}

impl RuntimeProfileSource {
    pub(crate) fn new(profile_dir: impl Into<PathBuf>) -> Self {
        Self {
            profile_dir: profile_dir.into(),
        }
    }

    pub(crate) fn profile_dir(&self) -> &Path {
        &self.profile_dir
    }

    pub(crate) fn load(&self) -> Result<RuntimeProfileConfig> {
        let profile = Profile::load_from_dir(&self.profile_dir)
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("load runtime profile {}", self.profile_dir.display()))?;
        RuntimeProfileConfig::from_profile(profile)
    }
}

impl RuntimeProfileConfig {
    fn from_profile(profile: Profile) -> Result<Self> {
        let config = profile.config();
        let security_rules = config
            .compile_security_rule_set_from_files(profile.config_root(), SecurityRuleSource::User)
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("compile runtime profile rules for {}", config.id))?;

        let mut plugins = ProviderRuleProfile::builtin_security_defaults().plugins;
        for (plugin_id, config) in &config.plugins {
            plugins.insert(plugin_id.clone(), *config);
        }

        let provider_profile = ProviderRuleProfile::merge_override(
            &ProviderRuleProfile::builtin_defaults(),
            &ProviderRuleProfile {
                ai: config.ai.clone(),
            },
        )
        .map_err(anyhow::Error::msg)
        .with_context(|| format!("compile runtime profile AI providers for {}", config.id))?;
        let model_endpoints = provider_profile
            .endpoint_registry()
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("compile runtime profile model endpoints for {}", config.id))?;

        Ok(Self {
            profile_id: config.id.clone(),
            profile_dir: profile.profile_dir().to_path_buf(),
            config_root: profile.config_root().to_path_buf(),
            network: NetworkPolicy::new(),
            security_rules,
            plugins,
            model_endpoints,
            mcp: config.mcp.clone().unwrap_or_default(),
        })
    }

    pub(crate) fn mcp_servers(
        &self,
        builtin_binary: Option<&Path>,
        builtin_env: HashMap<String, String>,
    ) -> Vec<McpServerDef> {
        capsem_core::mcp::build_profile_server_list(&self.mcp, builtin_binary, builtin_env)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use capsem_core::net::policy_config::SecurityPluginMode;

    #[test]
    fn runtime_profile_source_loads_rules_plugins_mcp_without_settings() {
        let dir = tempfile::tempdir().unwrap();
        let config_root = dir.path().join("config");
        let profile_dir = config_root.join("profiles/code");
        std::fs::create_dir_all(&profile_dir).unwrap();
        std::fs::write(
            profile_dir.join("enforcement.toml"),
            r#"
[profiles.rules.runtime_http]
name = "runtime_http"
action = "allow"
priority = 10
match = 'http.host == "profile.example"'
"#,
        )
        .unwrap();

        std::fs::write(
            profile_dir.join("profile.toml"),
            r#"
id = "code"
name = "Code"
description = "Runtime test profile."
revision = "test.1"
refresh_policy = "24h"

[rule_files]
enforcement = "profiles/code/enforcement.toml"

[plugins.credential_broker]
mode = "rewrite"
detection_level = "informational"

[mcp.server_enabled]
local = false
"#,
        )
        .unwrap();

        let runtime = RuntimeProfileSource::new(&profile_dir).load().unwrap();

        assert_eq!(runtime.profile_id, "code");
        assert!(runtime
            .security_rules
            .rules()
            .iter()
            .any(|rule| rule.rule_id == "profiles.rules.runtime_http"));
        assert_eq!(
            runtime.plugins["credential_broker"].mode,
            SecurityPluginMode::Rewrite
        );
        assert_eq!(runtime.mcp.server_enabled["local"], false);
        assert_eq!(runtime.network.http_upstream_ports, vec![80, 3128, 3713, 8080, 11434]);
    }
}
