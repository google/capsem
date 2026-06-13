use anyhow::{Context, Result};
use capsem_core::mcp::types::McpServerDef;
use capsem_core::net::policy::NetworkPolicy;
use capsem_core::net::policy_config::{
    MergedPolicies, ModelEndpointRegistry, Profile, ProviderRuleProfile, SecurityPluginConfig,
    SecurityRuleSet, SecurityRuleSource, SettingsFile,
};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

const RUNTIME_OVERLAY_FILE: &str = "runtime-overlay.toml";

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
        let runtime_overlay = load_runtime_overlay(profile.profile_dir())?;
        let profile_rules = config
            .compile_security_rule_set_from_files(profile.config_root(), SecurityRuleSource::User)
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("compile runtime profile rules for {}", config.id))?;
        let profile_rule_settings = SettingsFile {
            ai: config.ai.clone(),
            ..SettingsFile::default()
        };
        let overlay_policies = MergedPolicies::from_files(&profile_rule_settings, &runtime_overlay);
        let mut rules_by_id = BTreeMap::new();
        for rule in profile_rules.rules() {
            rules_by_id.insert(rule.rule_id.clone(), rule.clone());
        }
        for rule in overlay_policies.security_rules.rules() {
            rules_by_id.insert(rule.rule_id.clone(), rule.clone());
        }
        let security_rules = SecurityRuleSet::new(rules_by_id.into_values().collect());

        let mut plugins = ProviderRuleProfile::builtin_security_defaults().plugins;
        for (plugin_id, config) in &config.plugins {
            plugins.insert(plugin_id.clone(), *config);
        }
        for (plugin_id, config) in &runtime_overlay.plugins {
            plugins.insert(plugin_id.clone(), *config);
        }

        let provider_profile = ProviderRuleProfile::merge_defaults_user_and_corp(
            &ProviderRuleProfile {
                ai: config.ai.clone(),
            },
            &ProviderRuleProfile {
                ai: runtime_overlay.ai.clone(),
            },
        )
        .map_err(anyhow::Error::msg)
        .with_context(|| format!("compile runtime profile AI providers for {}", config.id))?;
        let model_endpoints = provider_profile
            .endpoint_registry()
            .map_err(anyhow::Error::msg)
            .with_context(|| {
                format!("compile runtime profile model endpoints for {}", config.id)
            })?;

        Ok(Self {
            profile_id: config.id.clone(),
            profile_dir: profile.profile_dir().to_path_buf(),
            config_root: profile.config_root().to_path_buf(),
            network: overlay_policies.network,
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

fn load_runtime_overlay(profile_dir: &Path) -> Result<SettingsFile> {
    let path = profile_dir.join(RUNTIME_OVERLAY_FILE);
    if !path.exists() {
        return Ok(SettingsFile::default());
    }
    let content =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let overlay: SettingsFile =
        toml::from_str(&content).with_context(|| format!("parse {}", path.display()))?;
    overlay
        .validate_metadata_contract()
        .map_err(anyhow::Error::msg)
        .with_context(|| format!("validate {}", path.display()))?;
    Ok(overlay)
}

#[cfg(test)]
mod tests {
    use super::*;
    use capsem_core::net::policy_config::SecurityPluginMode;

    #[test]
    fn runtime_profile_source_loads_profile_rules_plugins_mcp() {
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
        assert!(!runtime.mcp.server_enabled["local"]);
        assert_eq!(
            runtime.network.http_upstream_ports,
            vec![80, 3128, 3713, 8080, 11434]
        );
    }

    #[test]
    fn runtime_profile_source_loads_service_supplied_corp_overlay_without_global_config() {
        let dir = tempfile::tempdir().unwrap();
        let config_root = dir.path().join("config");
        let profile_dir = config_root.join("profiles/code");
        std::fs::create_dir_all(&profile_dir).unwrap();
        std::fs::write(
            profile_dir.join("profile.toml"),
            r#"
id = "code"
name = "Code"
description = "Runtime test profile."
revision = "test.1"
refresh_policy = "24h"

[default.http]
name = "default_http"
action = "allow"
priority = "default"
match = 'has(http.host)'
"#,
        )
        .unwrap();
        std::fs::write(
            profile_dir.join(RUNTIME_OVERLAY_FILE),
            r#"
[corp.rules.block_local_deny_target]
name = "block_local_deny_target"
action = "block"
priority = -100
detection_level = "high"
match = 'http.host == "127.0.0.1" && http.path == "/deny-target"'
"#,
        )
        .unwrap();

        let runtime = RuntimeProfileSource::new(&profile_dir).load().unwrap();
        let event = serde_json::json!({
            "http": {
                "host": "127.0.0.1",
                "path": "/deny-target"
            }
        });
        let evaluation = runtime.security_rules.evaluate(&event).unwrap();
        let first = evaluation
            .enforcement_rules()
            .into_iter()
            .next()
            .expect("corp rule should match");

        assert_eq!(first.rule_id, "corp.rules.block_local_deny_target");
        assert_eq!(
            first.action,
            capsem_core::net::policy_config::SecurityRuleAction::Block
        );
    }
}
