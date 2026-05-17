use std::path::{Path, PathBuf};
use std::sync::Arc;

use capsem_core::mcp::aggregator::AggregatorClient;
use capsem_core::mcp::policy::McpPolicy;
use capsem_core::mcp::policy::McpUserConfig;
use capsem_core::mcp::policy::ToolDecision;
use capsem_core::mcp::types::McpServerDef;
use capsem_core::net::domain_policy::{Action, DomainPolicy};
use capsem_core::net::policy::NetworkPolicy;
use capsem_core::net::policy_config::{
    GuestConfig, PolicyCallback, PolicyConfig, PolicyDecisionKind, PolicyRuleConfig,
};
use capsem_core::settings_profiles::{
    self, CapabilityMode, EffectiveRule, RuleDecision, VmNetworkMode,
};
use std::collections::HashMap;
use tracing::warn;

const DEFAULT_SNAPSHOT_AUTO_MAX: usize = 10;
const DEFAULT_SNAPSHOT_MANUAL_MAX: usize = 12;
const DEFAULT_SNAPSHOT_INTERVAL_SECS: u64 = 300;

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

#[derive(Clone)]
pub(crate) struct RuntimePolicyState {
    pub(crate) guest_config: GuestConfig,
    pub(crate) network_policy: NetworkPolicy,
    pub(crate) domain_policy: DomainPolicy,
    pub(crate) mcp_policy: McpPolicy,
    pub(crate) policy_v2: PolicyConfig,
    pub(crate) mcp_user: McpUserConfig,
    pub(crate) mcp_corp: McpUserConfig,
    pub(crate) snapshot_auto_max: usize,
    pub(crate) snapshot_manual_max: usize,
    pub(crate) snapshot_interval_secs: u64,
}

pub(crate) fn load_runtime_policy_state(session_dir: &Path) -> RuntimePolicyState {
    let effective = load_effective_vm_settings_with_fallback(session_dir);

    let mut default_allow = match effective.as_ref().map(|e| e.vm.value.network) {
        Some(VmNetworkMode::Disabled) => false,
        Some(VmNetworkMode::Proxied | VmNetworkMode::Direct) | None => true,
    };
    if matches!(
        effective
            .as_ref()
            .map(|e| e.security.value.capabilities.network_egress),
        Some(CapabilityMode::Block)
    ) {
        default_allow = false;
    }

    let network_policy = NetworkPolicy::new(Vec::new(), default_allow, default_allow);
    let domain_policy = DomainPolicy::new(
        &[],
        &[],
        if default_allow {
            Action::Allow
        } else {
            Action::Deny
        },
    );

    let mcp_user = effective
        .as_ref()
        .map(mcp_user_config_from_effective)
        .unwrap_or_default();
    let mcp_corp = McpUserConfig::default();
    let mcp_policy = mcp_user.to_policy(&mcp_corp);
    let policy_v2 = effective
        .as_ref()
        .map(policy_v2_from_effective_rules)
        .unwrap_or_default();

    RuntimePolicyState {
        guest_config: GuestConfig::default(),
        network_policy,
        domain_policy,
        mcp_policy,
        policy_v2,
        mcp_user,
        mcp_corp,
        snapshot_auto_max: DEFAULT_SNAPSHOT_AUTO_MAX,
        snapshot_manual_max: DEFAULT_SNAPSHOT_MANUAL_MAX,
        snapshot_interval_secs: DEFAULT_SNAPSHOT_INTERVAL_SECS,
    }
}

fn load_effective_vm_settings_with_fallback(
    session_dir: &Path,
) -> Option<settings_profiles::EffectiveVmSettings> {
    match settings_profiles::load_vm_effective_settings(session_dir) {
        Ok(effective) => Some(effective),
        Err(error) => {
            warn!(
                error = %error,
                session_dir = %session_dir.display(),
                "failed to load vm-effective settings attachment; falling back to default profile"
            );
            let defaults = settings_profiles::ProfileRootSettings::default();
            match settings_profiles::resolve_effective_vm_settings(&defaults, None) {
                Ok(effective) => Some(effective),
                Err(resolve_error) => {
                    warn!(
                        error = %resolve_error,
                        "failed to resolve fallback default profile; running with open runtime policies"
                    );
                    None
                }
            }
        }
    }
}

fn mcp_user_config_from_effective(
    effective: &settings_profiles::EffectiveVmSettings,
) -> McpUserConfig {
    let default_tool_permission = Some(match effective.security.value.capabilities.mcp_tools {
        CapabilityMode::Allow | CapabilityMode::Audit => ToolDecision::Allow,
        CapabilityMode::Ask => ToolDecision::Warn,
        CapabilityMode::Block => ToolDecision::Block,
    });

    let server_enabled = effective
        .mcp
        .value
        .connectors
        .iter()
        .map(|(id, connector)| (id.clone(), connector.enabled))
        .collect::<HashMap<_, _>>();

    let mut tool_permissions = HashMap::new();
    for rule in &effective.rules {
        if rule.derived || rule.callback != "mcp.request" {
            continue;
        }
        let Some(tool_name) = mcp_tool_name_from_condition(&rule.condition) else {
            continue;
        };
        let decision = match rule.decision {
            RuleDecision::Allow => ToolDecision::Allow,
            RuleDecision::Ask => ToolDecision::Warn,
            RuleDecision::Block => ToolDecision::Block,
            RuleDecision::Rewrite => continue,
        };
        tool_permissions.entry(tool_name).or_insert(decision);
    }

    McpUserConfig {
        global_policy: None,
        default_tool_permission,
        health_check_interval_secs: None,
        servers: Vec::new(),
        server_enabled,
        tool_permissions,
    }
}

fn policy_v2_from_effective_rules(
    effective: &settings_profiles::EffectiveVmSettings,
) -> PolicyConfig {
    let mut config = PolicyConfig::default();
    for (index, rule) in effective.rules.iter().enumerate() {
        if rule.derived {
            continue;
        }
        let Some(callback) = map_effective_callback(&rule.callback) else {
            warn!(
                rule_id = %rule.id,
                callback = %rule.callback,
                "skipping unsupported effective rule callback for current runtime policy engine"
            );
            continue;
        };
        let rule_name = effective_rule_name(rule, index);
        let policy_rule = PolicyRuleConfig {
            on: callback,
            condition: rule.condition.clone(),
            decision: map_rule_decision(rule.decision),
            priority: rule.priority,
            reason: rule.reason.clone(),
            rewrite_target: rule.rewrite_target.clone(),
            rewrite_value: rule.rewrite_value.clone(),
            strip_request_headers: Vec::new(),
            strip_response_headers: Vec::new(),
        };
        if let Err(error) = policy_rule.validate() {
            warn!(
                rule_id = %rule.id,
                callback = %rule.callback,
                error = %error,
                "skipping invalid effective rule during process policy conversion"
            );
            continue;
        }
        policy_rules_mut(&mut config, callback).insert(rule_name, policy_rule);
    }
    config
}

fn effective_rule_name(rule: &EffectiveRule, index: usize) -> String {
    rule.id
        .split_once('.')
        .map(|(_, name)| name.to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| format!("rule-{index}"))
}

fn map_effective_callback(callback: &str) -> Option<PolicyCallback> {
    match callback {
        "mcp.request" => Some(PolicyCallback::McpRequest),
        "mcp.response" => Some(PolicyCallback::McpResponse),
        "http.request" => Some(PolicyCallback::HttpRequest),
        "http.response" => Some(PolicyCallback::HttpResponse),
        "dns.request" => Some(PolicyCallback::DnsQuery),
        "dns.response" => Some(PolicyCallback::DnsResponse),
        "model.request" => Some(PolicyCallback::ModelRequest),
        "model.response" => Some(PolicyCallback::ModelResponse),
        "model.tool_call" => Some(PolicyCallback::ModelToolCall),
        "model.tool_response" => Some(PolicyCallback::ModelToolResponse),
        "hook.decision" => Some(PolicyCallback::HookDecision),
        _ => None,
    }
}

fn map_rule_decision(decision: RuleDecision) -> PolicyDecisionKind {
    match decision {
        RuleDecision::Allow => PolicyDecisionKind::Allow,
        RuleDecision::Ask => PolicyDecisionKind::Ask,
        RuleDecision::Block => PolicyDecisionKind::Block,
        RuleDecision::Rewrite => PolicyDecisionKind::Rewrite,
    }
}

fn policy_rules_mut(
    config: &mut PolicyConfig,
    callback: PolicyCallback,
) -> &mut HashMap<String, PolicyRuleConfig> {
    match callback {
        PolicyCallback::McpRequest | PolicyCallback::McpResponse => &mut config.mcp,
        PolicyCallback::HttpRequest | PolicyCallback::HttpResponse => &mut config.http,
        PolicyCallback::DnsQuery | PolicyCallback::DnsResponse => &mut config.dns,
        PolicyCallback::ModelRequest
        | PolicyCallback::ModelResponse
        | PolicyCallback::ModelToolCall
        | PolicyCallback::ModelToolResponse => &mut config.model,
        PolicyCallback::HookDecision => &mut config.hook,
    }
}

fn mcp_tool_name_from_condition(condition: &str) -> Option<String> {
    let tool_idx = condition.find("tool.name")?;
    let after_name = &condition[tool_idx + "tool.name".len()..];
    let eq_idx = after_name.find("==")?;
    let value = after_name[eq_idx + 2..].trim_start();
    let mut chars = value.chars();
    let quote = chars.next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let tail = &value[quote.len_utf8()..];
    let end = tail.find(quote)?;
    let name = tail[..end].trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
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
