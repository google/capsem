use anyhow::{Context, Result};
use capsem_core::mcp::types::McpServerDef;
use capsem_core::net::policy::NetworkPolicy;
use capsem_core::net::policy_config::{
    ActiveProfileFile, MergedPolicies, ModelEndpointRegistry, SecurityPluginConfig, SecurityRuleSet,
};
use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub(crate) struct RuntimeProfileSource {
    active_profile_path: PathBuf,
}

#[derive(Debug, Clone)]
pub(crate) struct RuntimeProfileConfig {
    pub(crate) profile_id: String,
    pub(crate) active_profile_path: PathBuf,
    pub(crate) network: NetworkPolicy,
    pub(crate) dns_upstreams: Vec<SocketAddr>,
    pub(crate) security_rules: SecurityRuleSet,
    pub(crate) plugins: BTreeMap<String, SecurityPluginConfig>,
    pub(crate) model_endpoints: ModelEndpointRegistry,
    pub(crate) mcp: capsem_core::mcp::policy::McpUserConfig,
}

impl RuntimeProfileSource {
    pub(crate) fn new(active_profile_path: impl Into<PathBuf>) -> Self {
        Self {
            active_profile_path: active_profile_path.into(),
        }
    }

    pub(crate) fn active_profile_path(&self) -> &Path {
        &self.active_profile_path
    }

    pub(crate) fn load(&self) -> Result<RuntimeProfileConfig> {
        let content = std::fs::read_to_string(&self.active_profile_path)
            .with_context(|| format!("read {}", self.active_profile_path.display()))?;
        let active: ActiveProfileFile = toml::from_str(&content)
            .with_context(|| format!("parse {}", self.active_profile_path.display()))?;
        RuntimeProfileConfig::from_active(active, self.active_profile_path.clone())
    }
}

impl RuntimeProfileConfig {
    fn from_active(active: ActiveProfileFile, active_profile_path: PathBuf) -> Result<Self> {
        active
            .validate()
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("validate {}", active_profile_path.display()))?;
        let (profile_settings, corp_settings) = active.merged_policy_inputs();
        let merged = MergedPolicies::from_files(&profile_settings, &corp_settings);
        let mut network = merged.network;
        active.network.apply_to_policy(&mut network);
        let security_rules = active
            .compile_security_rule_set()
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("compile active profile rules for {}", active.id))?;
        let model_endpoints = active
            .model_endpoint_registry()
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("compile active profile model endpoints for {}", active.id))?;
        let dns_upstreams = active
            .network
            .dns
            .upstreams
            .iter()
            .map(|upstream| {
                upstream.parse::<SocketAddr>().with_context(|| {
                    format!(
                        "parse DNS upstream {upstream:?} from {}",
                        active_profile_path.display()
                    )
                })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(Self {
            profile_id: active.id.clone(),
            active_profile_path,
            network,
            dns_upstreams,
            security_rules,
            plugins: active.plugins.clone(),
            model_endpoints,
            mcp: active.mcp.clone().unwrap_or_default(),
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
    fn runtime_profile_source_loads_active_profile_rules_plugins_mcp() {
        let dir = tempfile::tempdir().unwrap();
        let active_path = dir.path().join("vm/active_profile.toml");
        std::fs::create_dir_all(active_path.parent().unwrap()).unwrap();
        std::fs::write(
            &active_path,
            r#"
id = "code"
name = "Code"
description = "Runtime test active profile."
revision = "test.1"

[profile_rules.profiles.rules.runtime_http]
name = "runtime_http"
action = "allow"
priority = 10
match = 'http.host == "profile.example"'

[plugins.credential_broker]
mode = "rewrite"
detection_level = "informational"

[mcp.server_enabled]
local = false
"#,
        )
        .unwrap();

        let runtime = RuntimeProfileSource::new(&active_path).load().unwrap();

        assert_eq!(runtime.profile_id, "code");
        assert_eq!(runtime.active_profile_path, active_path);
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
    fn runtime_profile_source_loads_corp_rules_and_dns_from_active_profile() {
        let dir = tempfile::tempdir().unwrap();
        let active_path = dir.path().join("vm/active_profile.toml");
        std::fs::create_dir_all(active_path.parent().unwrap()).unwrap();
        std::fs::write(
            &active_path,
            r#"
id = "code"
name = "Code"
description = "Runtime test active profile."
revision = "test.1"

[profile_rules.default.http]
name = "default_http"
action = "allow"
priority = "default"
match = 'has(http.host)'

[corp_rules.corp.rules.block_local_deny_target]
name = "block_local_deny_target"
action = "block"
priority = -100
detection_level = "high"
match = 'http.host == "127.0.0.1" && http.path == "/deny-target"'

[network]
log_bodies = true
max_body_capture = 8192
http_upstream_ports = [80, 3713]

[network.dns]
upstreams = ["127.0.0.1:5353"]
"#,
        )
        .unwrap();

        let runtime = RuntimeProfileSource::new(&active_path).load().unwrap();
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
            runtime.dns_upstreams,
            vec!["127.0.0.1:5353".parse().unwrap()]
        );
        assert!(runtime.network.log_bodies);
        assert_eq!(runtime.network.max_body_capture, 8192);
        assert_eq!(runtime.network.http_upstream_ports, vec![80, 3713]);
        assert_eq!(
            first.action,
            capsem_core::net::policy_config::SecurityRuleAction::Block
        );
    }
}
