use anyhow::{Context, Result};
use capsem_core::net::policy::NetworkMechanics;
use capsem_core::net::policy_config::{
    ActiveProfileFile, MergedPolicies, ModelEndpointRegistry, SecurityPluginConfig, SecurityRuleSet,
};
use capsem_proto::mcp_contracts::McpServerDef;
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
    pub(crate) network: NetworkMechanics,
    pub(crate) dns_upstreams: Vec<SocketAddr>,
    pub(crate) security_rules: SecurityRuleSet,
    pub(crate) plugins: BTreeMap<String, SecurityPluginConfig>,
    pub(crate) model_endpoints: ModelEndpointRegistry,
    pub(crate) mcp: capsem_core::mcp::policy::McpProfileConfig,
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
        let active: ActiveProfileFile =
            toml::from_str(&content).with_context(|| format!("parse {}", self.active_profile_path.display()))?;
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
        capsem_core::net::policy_config::apply_network_config(&active.network, &mut network);
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
                upstream
                    .parse::<SocketAddr>()
                    .with_context(|| format!("parse DNS upstream {upstream:?} from {}", active_profile_path.display()))
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
mod tests;
