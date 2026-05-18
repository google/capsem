use std::path::{Path, PathBuf};
use std::sync::Arc;

use capsem_core::mcp::aggregator::AggregatorClient;
use capsem_core::mcp::policy::McpPolicy;
use capsem_core::mcp::policy::McpUserConfig;
use capsem_core::mcp::policy::ToolDecision;
use capsem_core::mcp::types::McpServerDef;
use capsem_core::net::domain_policy::{Action, DomainPolicy};
use capsem_core::net::policy::{DomainMatcher, NetworkPolicy, PolicyRule};
use capsem_core::net::policy_config::{
    PolicyCallback, PolicyConfig, PolicyDecisionKind, PolicyRuleConfig,
};
use capsem_core::settings_profiles::{
    self, CapabilityMode, EffectiveRule, RuleDecision, VmNetworkMode,
};
use capsem_core::vm::guest_config::{GuestConfig, GuestFile};
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
    load_runtime_policy_state_from_effective(session_dir)
}

fn load_runtime_policy_state_from_effective(session_dir: &Path) -> RuntimePolicyState {
    let effective = load_effective_vm_settings_with_fallback(session_dir);

    let (default_allow_read, default_allow_write) =
        network_defaults_from_effective(effective.as_ref());
    let network_policy = NetworkPolicy::new(
        network_policy_rules_from_effective(effective.as_ref()),
        default_allow_read,
        default_allow_write,
    );
    let domain_default_allow = effective
        .as_ref()
        .map(|effective| {
            matches!(
                effective.security.value.capabilities.network_egress,
                CapabilityMode::Allow | CapabilityMode::Audit
            )
        })
        .unwrap_or(false);
    let (domain_allow, domain_block) = domain_policy_lists_from_effective(effective.as_ref());
    let domain_policy = DomainPolicy::new(
        &domain_allow,
        &domain_block,
        if domain_default_allow {
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
    let guest_config = guest_config_from_effective(effective.as_ref());

    RuntimePolicyState {
        guest_config,
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

fn network_defaults_from_effective(
    effective: Option<&settings_profiles::EffectiveVmSettings>,
) -> (bool, bool) {
    if matches!(
        effective.map(|effective| effective.vm.value.network),
        Some(VmNetworkMode::Disabled)
    ) {
        return (false, false);
    }

    match effective
        .map(|effective| effective.security.value.capabilities.network_egress)
        .unwrap_or(CapabilityMode::Ask)
    {
        CapabilityMode::Allow | CapabilityMode::Audit => (true, true),
        CapabilityMode::Ask => (true, false),
        CapabilityMode::Block => (false, false),
    }
}

fn guest_config_from_effective(
    effective: Option<&settings_profiles::EffectiveVmSettings>,
) -> GuestConfig {
    let (default_allow_read, default_allow_write) = network_defaults_from_effective(effective);

    let provider_allowed = |name: &str| {
        effective
            .and_then(|effective| effective.ai.value.providers.get(name))
            .map(|provider| provider.enabled)
            .unwrap_or(default_allow_read)
    };

    let mut env = HashMap::new();
    env.insert(
        "REQUESTS_CA_BUNDLE".to_string(),
        "/etc/ssl/certs/ca-certificates.crt".to_string(),
    );
    env.insert(
        "NODE_EXTRA_CA_CERTS".to_string(),
        "/etc/ssl/certs/ca-certificates.crt".to_string(),
    );
    env.insert(
        "SSL_CERT_FILE".to_string(),
        "/etc/ssl/certs/ca-certificates.crt".to_string(),
    );
    env.insert("TERM".to_string(), "xterm-256color".to_string());
    env.insert("HOME".to_string(), "/root".to_string());
    env.insert(
        "PATH".to_string(),
        "/opt/ai-clis/bin:/root/.local/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_string(),
    );
    env.insert("LANG".to_string(), "C".to_string());
    env.insert(
        "CAPSEM_WEB_ALLOW_READ".to_string(),
        if default_allow_read { "1" } else { "0" }.to_string(),
    );
    env.insert(
        "CAPSEM_WEB_ALLOW_WRITE".to_string(),
        if default_allow_write { "1" } else { "0" }.to_string(),
    );
    env.insert(
        "CAPSEM_OPENAI_ALLOWED".to_string(),
        if provider_allowed("openai") { "1" } else { "0" }.to_string(),
    );
    env.insert(
        "CAPSEM_ANTHROPIC_ALLOWED".to_string(),
        if provider_allowed("anthropic") {
            "1"
        } else {
            "0"
        }
        .to_string(),
    );
    env.insert(
        "CAPSEM_GOOGLE_ALLOWED".to_string(),
        if provider_allowed("google") { "1" } else { "0" }.to_string(),
    );

    let files = vec![
        GuestFile {
            path: "/root/.gemini/settings.json".to_string(),
            content: r#"{"homeDirectoryWarningDismissed":true,"general":{"disableAutoUpdate":true,"disableUpdateNag":true},"ui":{"hideTips":true,"hideBanner":false},"privacy":{"usageStatisticsEnabled":false,"sessionRetention":"none"},"telemetry":{"enabled":false},"security":{"auth":{"selectedType":"gemini-api-key"},"folderTrust.enabled":false},"ide":{"hasSeenNudge":true},"tools":{"sandbox":false},"mcpServers":{"local":{"command":"/run/capsem-mcp-server"}}}"#.to_string(),
            mode: 0o600,
        },
        GuestFile {
            path: "/root/.gemini/installation_id".to_string(),
            content: "capsem-sandbox-00000000-0000-0000-0000-000000000000".to_string(),
            mode: 0o600,
        },
        GuestFile {
            path: "/root/.gemini/projects.json".to_string(),
            content: r#"{"projects":{"/root":"root"}}"#.to_string(),
            mode: 0o600,
        },
        GuestFile {
            path: "/root/.gemini/trustedFolders.json".to_string(),
            content: r#"{"/root":"TRUST_FOLDER"}"#.to_string(),
            mode: 0o600,
        },
        GuestFile {
            path: "/root/.codex/config.toml".to_string(),
            content: "[mcp_servers.local]\ncommand = \"/run/capsem-mcp-server\"\n".to_string(),
            mode: 0o600,
        },
        GuestFile {
            path: "/root/.claude/settings.json".to_string(),
            content: r#"{"permissions":{"defaultMode":"bypassPermissions"},"env":{"CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC":"1"},"mcpServers":{"local":{"command":"/run/capsem-mcp-server"}}}"#.to_string(),
            mode: 0o600,
        },
        GuestFile {
            path: "/root/.claude.json".to_string(),
            content: r#"{"hasCompletedOnboarding":true,"hasTrustDialogAccepted":true,"hasTrustDialogHooksAccepted":true,"shiftEnterKeyBindingInstalled":true,"theme":"dark","numStartups":1,"opusProMigrationComplete":true,"sonnet1m45MigrationComplete":true,"projects":{"/root":{"allowedTools":[],"hasTrustDialogAccepted":true,"projectOnboardingSeenCount":1}},"mcpServers":{"local":{"command":"/run/capsem-mcp-server"}}}"#.to_string(),
            mode: 0o600,
        },
    ];

    GuestConfig {
        env: Some(env),
        files: Some(files),
    }
}

fn domain_policy_lists_from_effective(
    effective: Option<&settings_profiles::EffectiveVmSettings>,
) -> (Vec<String>, Vec<String>) {
    let mut allow = Vec::new();
    let mut block = Vec::new();
    let Some(effective) = effective else {
        return (allow, block);
    };

    for rule in &effective.rules {
        let Some(domain) = domain_from_simple_network_condition(rule) else {
            continue;
        };
        match rule.decision {
            RuleDecision::Allow => push_unique(&mut allow, domain),
            RuleDecision::Ask | RuleDecision::Block => push_unique(&mut block, domain),
            RuleDecision::Rewrite => {}
        }
    }
    (allow, block)
}

fn network_policy_rules_from_effective(
    effective: Option<&settings_profiles::EffectiveVmSettings>,
) -> Vec<PolicyRule> {
    let Some(effective) = effective else {
        return Vec::new();
    };

    let mut rules = effective
        .rules
        .iter()
        .enumerate()
        .filter_map(|(index, rule)| {
            let domain = domain_from_simple_network_condition(rule)?;
            let (allow_read, allow_write) = match rule.decision {
                RuleDecision::Allow => (true, true),
                RuleDecision::Ask | RuleDecision::Block => (false, false),
                RuleDecision::Rewrite => return None,
            };
            Some((
                rule.priority,
                index,
                PolicyRule {
                    matcher: DomainMatcher::parse(&domain),
                    allow_read,
                    allow_write,
                },
            ))
        })
        .collect::<Vec<_>>();

    rules.sort_by_key(|(priority, index, _)| (*priority, *index));
    rules.into_iter().map(|(_, _, rule)| rule).collect()
}

fn domain_from_simple_network_condition(rule: &EffectiveRule) -> Option<String> {
    match rule.callback.as_str() {
        "dns.request" => extract_condition_eq(&rule.condition, "qname"),
        "http.request" | "http.read" | "http.write" | "http.response" => {
            extract_condition_eq(&rule.condition, "request.host")
        }
        _ => None,
    }
}

fn extract_condition_eq(condition: &str, field: &str) -> Option<String> {
    for quote in ['"', '\''] {
        let prefix = format!("{field} == {quote}");
        if let Some(rest) = condition.trim().strip_prefix(&prefix) {
            let end = rest.find(quote)?;
            if !rest[end + quote.len_utf8()..].trim().is_empty() {
                continue;
            }
            let value = rest[..end].trim();
            if !value.is_empty() {
                return Some(value.to_ascii_lowercase());
            }
        }
    }
    None
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.iter().any(|existing| existing == &value) {
        values.push(value);
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
            strip_request_headers: rule.strip_request_headers.clone(),
            strip_response_headers: rule.strip_response_headers.clone(),
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
    let condition = condition.trim();
    let after_name = condition.strip_prefix("tool.name")?;
    let eq_idx = after_name.find("==")?;
    let value = after_name[eq_idx + 2..].trim_start();
    let mut chars = value.chars();
    let quote = chars.next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let tail = &value[quote.len_utf8()..];
    let end = tail.find(quote)?;
    if !tail[end + quote.len_utf8()..].trim().is_empty() {
        return None;
    }
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
    env.insert(
        "CAPSEM_DOMAIN_DEFAULT".to_string(),
        match policy.default_action() {
            Action::Allow => "allow",
            Action::Deny => "deny",
        }
        .to_string(),
    );

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
