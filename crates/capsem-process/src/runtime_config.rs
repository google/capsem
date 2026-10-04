use anyhow::{Context, Result};
use capsem_core::net::policy::NetworkMechanics;
use capsem_core::net::policy_config::{
    ActivePolicyFile, MergedPolicies, ModelEndpointRegistry, SecurityPluginConfig, SecurityRuleSet,
};
use capsem_proto::mcp_contracts::McpServerDef;
use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

/// The session's active policy file, read at boot and on every reload.
#[derive(Debug, Clone)]
pub(crate) struct RuntimePolicySource {
    active_policy_path: PathBuf,
}

#[derive(Debug, Clone)]
pub(crate) struct RuntimePolicyConfig {
    pub(crate) active_policy_path: PathBuf,
    /// Digest of the exact bytes this config was loaded from.
    pub(crate) active_policy_digest: String,
    pub(crate) network: NetworkMechanics,
    pub(crate) dns_upstreams: Vec<SocketAddr>,
    pub(crate) security_rules: SecurityRuleSet,
    pub(crate) plugins: BTreeMap<String, SecurityPluginConfig>,
    pub(crate) model_endpoints: ModelEndpointRegistry,
    pub(crate) mcp: capsem_core::mcp::policy::McpConfig,
}

impl RuntimePolicySource {
    pub(crate) fn new(active_policy_path: impl Into<PathBuf>) -> Self {
        Self {
            active_policy_path: active_policy_path.into(),
        }
    }

    pub(crate) fn active_policy_path(&self) -> &Path {
        &self.active_policy_path
    }

    pub(crate) fn load(&self) -> Result<RuntimePolicyConfig> {
        let content = std::fs::read_to_string(&self.active_policy_path)
            .with_context(|| format!("read {}", self.active_policy_path.display()))?;
        let active: ActivePolicyFile =
            toml::from_str(&content).with_context(|| format!("parse {}", self.active_policy_path.display()))?;
        let digest = capsem_core::net::policy_config::active_policy_digest(content.as_bytes());
        RuntimePolicyConfig::from_active(active, self.active_policy_path.clone(), digest)
    }
}

impl RuntimePolicyConfig {
    fn from_active(
        active: ActivePolicyFile,
        active_policy_path: PathBuf,
        active_policy_digest: String,
    ) -> Result<Self> {
        let path = active_policy_path.display().to_string();
        active
            .validate()
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("validate {path}"))?;
        let (user_settings, corp_settings) = active.merged_policy_inputs();
        let merged = MergedPolicies::from_files(&user_settings, &corp_settings)
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("merge active policy {path}"))?;
        let mut network = merged.network;
        capsem_core::net::policy_config::apply_network_config(&active.network, &mut network);
        let security_rules = active
            .compile_security_rule_set()
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("compile active policy rules from {path}"))?;
        let model_endpoints = active
            .model_endpoint_registry()
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("compile active policy model endpoints from {path}"))?;
        let dns_upstreams = active
            .network
            .dns
            .upstreams
            .iter()
            .map(|upstream| {
                upstream
                    .parse::<SocketAddr>()
                    .with_context(|| format!("parse DNS upstream {upstream:?} from {path}"))
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(Self {
            active_policy_path,
            active_policy_digest,
            network,
            dns_upstreams,
            security_rules,
            plugins: active.plugins.clone(),
            model_endpoints,
            mcp: active.mcp.clone().unwrap_or_default(),
        })
    }

    /// Put this policy in force for traffic, MCP and model routing.
    pub(crate) fn apply(
        self,
        net_state: &capsem_core::SandboxNetworkState,
        mcp_runtime: &crate::mcp_runtime::McpRuntime,
    ) {
        let security_rule_ids = self
            .security_rules
            .rules()
            .iter()
            .map(|rule| rule.rule_id.clone())
            .collect::<Vec<_>>();
        *net_state.policy.write().unwrap() = std::sync::Arc::new(self.network);
        *mcp_runtime.security_rules.write().unwrap() = std::sync::Arc::new(self.security_rules);
        *mcp_runtime.plugin_policy.write().unwrap() = std::sync::Arc::new(self.plugins);
        *mcp_runtime.model_endpoints.write().unwrap() = std::sync::Arc::new(self.model_endpoints);
        tracing::info!(
            active_policy_digest = %self.active_policy_digest,
            security_rule_count = security_rule_ids.len(),
            security_rule_ids = ?security_rule_ids,
            "Reloaded runtime policy"
        );
    }

    pub(crate) fn mcp_servers(
        &self,
        builtin_binary: Option<&Path>,
        builtin_env: HashMap<String, String>,
    ) -> Vec<McpServerDef> {
        capsem_core::mcp::build_server_list(&self.mcp, builtin_binary, builtin_env)
    }
}

#[cfg(test)]
mod tests;
