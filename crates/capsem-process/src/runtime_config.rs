use anyhow::{Context, Result};
use capsem_core::net::policy::NetworkMechanics;
use capsem_core::net::policy_config::{ActivePolicyFile, ModelEndpointRegistry, SecurityPluginConfig, SecurityRuleSet};
use capsem_proto::mcp_contracts::McpServerDef;
use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

/// The session's active policy file, read at boot. Reloads carry the service's
/// exact newly published bytes over authenticated IPC.
#[derive(Debug, Clone)]
pub(crate) struct RuntimePolicySource {
    active_policy_path: PathBuf,
    current: Arc<RwLock<Option<RuntimePolicyConfig>>>,
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
            current: Arc::new(RwLock::new(None)),
        }
    }

    pub(crate) fn active_policy_path(&self) -> &Path {
        &self.active_policy_path
    }

    pub(crate) fn load(&self) -> Result<RuntimePolicyConfig> {
        let content = std::fs::read(&self.active_policy_path)
            .with_context(|| format!("read {}", self.active_policy_path.display()))?;
        self.load_bytes(&content)
    }

    pub(crate) fn load_bytes(&self, content: &[u8]) -> Result<RuntimePolicyConfig> {
        let content_text =
            std::str::from_utf8(content).with_context(|| format!("decode {}", self.active_policy_path.display()))?;
        let active: ActivePolicyFile =
            toml::from_str(content_text).with_context(|| format!("parse {}", self.active_policy_path.display()))?;
        let digest = capsem_core::net::policy_config::active_policy_digest(content);
        let config = RuntimePolicyConfig::from_active(active, self.active_policy_path.clone(), digest)?;
        *self.current.write().unwrap() = Some(config.clone());
        Ok(config)
    }

    /// Return the last authenticated policy snapshot loaded at boot or over
    /// service IPC. The confined owner cannot reopen the coordinator-owned
    /// policy path after startup.
    pub(crate) fn current(&self) -> Result<RuntimePolicyConfig> {
        self.current
            .read()
            .unwrap()
            .clone()
            .context("runtime policy has not been loaded")
    }
}

impl RuntimePolicyConfig {
    fn from_active(
        active: ActivePolicyFile,
        active_policy_path: PathBuf,
        active_policy_digest: String,
    ) -> Result<Self> {
        let path = active_policy_path.display().to_string();
        let compiled = active
            .compile_runtime()
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("compile active policy {path}"))?;

        Ok(Self {
            active_policy_path,
            active_policy_digest,
            network: compiled.network,
            dns_upstreams: compiled.dns_upstreams,
            security_rules: compiled.security_rules,
            plugins: compiled.plugins,
            model_endpoints: compiled.model_endpoints,
            mcp: compiled.mcp,
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
        mcp_runtime.proxy_policy.replace(self.proxy_policy_snapshot());
        *net_state.policy.write().unwrap() = std::sync::Arc::new(self.network);
        *mcp_runtime.security_rules.write().unwrap() = std::sync::Arc::new(self.security_rules);
        *mcp_runtime.plugin_policy.write().unwrap() = std::sync::Arc::new(self.plugins);
        tracing::info!(
            active_policy_digest = %self.active_policy_digest,
            security_rule_count = security_rule_ids.len(),
            security_rule_ids = ?security_rule_ids,
            "Reloaded runtime policy"
        );
    }

    pub(crate) fn proxy_policy_snapshot(&self) -> capsem_core::net::proxy_engine::ProxyPolicySnapshot {
        capsem_core::net::proxy_engine::ProxyPolicySnapshot::new(
            self.active_policy_digest.clone(),
            self.network.clone(),
            self.security_rules.clone(),
            self.plugins.clone(),
            self.model_endpoints.clone(),
        )
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
