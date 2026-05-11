use std::path::{Path, PathBuf};
use std::sync::Arc;

use capsem_core::mcp::aggregator::AggregatorClient;
use capsem_core::mcp::policy::McpPolicy;
use capsem_core::mcp::policy::McpUserConfig;
use capsem_core::mcp::types::McpServerDef;
use capsem_core::net::domain_policy::DomainPolicy;
use capsem_core::net::policy_config::PolicyConfig;
use std::collections::HashMap;

/// Shared MCP state for capsem-process after the guest transport cutover.
///
/// This is deliberately not a guest "gateway" config. Guest MCP traffic now
/// enters through the MITM framed endpoint on vsock:5002; this state is only
/// the in-process holder for aggregator access and live policy reload.
pub(crate) struct McpRuntime {
    pub(crate) aggregator: AggregatorClient,
    pub(crate) policy: Arc<tokio::sync::RwLock<Arc<McpPolicy>>>,
    pub(crate) policy_v2: Arc<tokio::sync::RwLock<Arc<PolicyConfig>>>,
    pub(crate) domain_policy: Arc<std::sync::RwLock<Arc<DomainPolicy>>>,
    pub(crate) session_dir: PathBuf,
    pub(crate) builtin_binary: Option<PathBuf>,
}

pub(crate) fn build_builtin_env(
    session_dir: &Path,
    policy: &DomainPolicy,
) -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert(
        "CAPSEM_SESSION_DIR".into(),
        session_dir.to_string_lossy().to_string(),
    );
    env.insert(
        "CAPSEM_SESSION_DB".into(),
        session_dir.join("session.db").to_string_lossy().to_string(),
    );
    insert_builtin_domain_policy_env(&mut env, policy);
    env
}

pub(crate) fn build_servers_with_builtin(
    user_mcp: &McpUserConfig,
    corp_mcp: &McpUserConfig,
    builtin_binary: Option<&Path>,
    session_dir: &Path,
    policy: &DomainPolicy,
) -> Vec<McpServerDef> {
    capsem_core::mcp::build_server_list_with_builtin(
        user_mcp,
        corp_mcp,
        builtin_binary,
        build_builtin_env(session_dir, policy),
    )
}

pub(crate) fn insert_builtin_domain_policy_env(
    env: &mut HashMap<String, String>,
    policy: &DomainPolicy,
) {
    let allowed = policy.allowed_patterns();
    if !allowed.is_empty() {
        env.insert("CAPSEM_DOMAIN_ALLOW".to_string(), allowed.join(","));
    }

    let blocked = policy.blocked_patterns();
    if !blocked.is_empty() {
        env.insert("CAPSEM_DOMAIN_BLOCK".to_string(), blocked.join(","));
    }
}

#[cfg(test)]
mod tests;
