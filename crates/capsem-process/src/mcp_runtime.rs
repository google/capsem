use std::path::{Path, PathBuf};
use std::sync::Arc;

use capsem_core::mcp::aggregator::AggregatorClient;
use capsem_core::mcp::policy::{McpManualServer, McpPolicy, McpUserConfig, ToolDecision};
use capsem_core::mcp::types::McpServerDef;
use capsem_core::net::domain_policy::{Action, DomainPolicy};
use capsem_core::net::mitm_proxy::{RuntimeSecurityEngine, RuntimeSecurityEngineSlot};
use capsem_core::settings_profiles::{
    self, CapabilityMode, EffectiveRule, RuleDecision, VmNetworkMode,
};
use capsem_core::vm::guest_config::{GuestConfig, GuestFile};
use capsem_security_engine::{
    CelEnforcementEvaluator, CelEnforcementRule, SecurityDecisionAction, SecurityEngine,
};
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use tracing::{info, warn};

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
    pub(crate) domain_policy: Arc<std::sync::RwLock<Arc<DomainPolicy>>>,
    pub(crate) security_engine: Arc<RuntimeSecurityEngineSlot>,
    pub(crate) rule_matches: RuntimeRuleMatchAccumulator,
    pub(crate) session_dir: PathBuf,
    pub(crate) builtin_binary: Option<PathBuf>,
}

#[derive(Clone, Default)]
pub(crate) struct RuntimeRuleMatchAccumulator {
    inner: Arc<Mutex<BTreeMap<String, RuntimeRuleMatchStats>>>,
}

#[derive(Clone, Default)]
struct RuntimeRuleMatchStats {
    match_count: u64,
    last_matched_event: Option<String>,
    last_matched_unix_ms: Option<u64>,
}

impl RuntimeRuleMatchAccumulator {
    pub(crate) fn drain(&self) -> Vec<capsem_proto::ipc::RuntimeRuleMatchSnapshot> {
        let mut matches = self.inner.lock().unwrap();
        let drained = std::mem::take(&mut *matches);
        drained
            .into_iter()
            .map(
                |(rule_id, stats)| capsem_proto::ipc::RuntimeRuleMatchSnapshot {
                    rule_id,
                    match_count: stats.match_count,
                    last_matched_event: stats.last_matched_event,
                    last_matched_unix_ms: stats.last_matched_unix_ms,
                },
            )
            .collect()
    }
}

impl capsem_security_engine::RuleMatchRecorder for RuntimeRuleMatchAccumulator {
    fn record_rule_match(
        &mut self,
        rule_id: &str,
        event_id: &str,
        timestamp_unix_ms: u64,
    ) -> Result<(), capsem_security_engine::SecurityEngineError> {
        let mut matches = self.inner.lock().map_err(|error| {
            capsem_security_engine::SecurityEngineError::PhaseFailed {
                phase: capsem_security_engine::SecurityEnginePhase::Detection,
                message: format!("runtime rule match accumulator lock poisoned: {error}"),
            }
        })?;
        let stats = matches.entry(rule_id.to_owned()).or_default();
        stats.match_count += 1;
        stats.last_matched_event = Some(event_id.to_owned());
        stats.last_matched_unix_ms = Some(timestamp_unix_ms);
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct RuntimePolicyState {
    pub(crate) profile_id: String,
    pub(crate) guest_config: GuestConfig,
    pub(crate) domain_policy: DomainPolicy,
    pub(crate) security_engine: Option<Arc<dyn RuntimeSecurityEngine>>,
    pub(crate) mcp_policy: McpPolicy,
    pub(crate) mcp_user: McpUserConfig,
    pub(crate) mcp_corp: McpUserConfig,
    pub(crate) snapshot_auto_max: usize,
    pub(crate) snapshot_manual_max: usize,
    pub(crate) snapshot_interval_secs: u64,
}

pub(crate) fn load_runtime_policy_state(session_dir: &Path) -> RuntimePolicyState {
    load_runtime_policy_state_with_runtime_rules(session_dir, None)
}

pub(crate) fn load_runtime_policy_state_with_runtime_rules(
    session_dir: &Path,
    runtime_rules: Option<&capsem_proto::ipc::RuntimeSecurityRulesSnapshot>,
) -> RuntimePolicyState {
    load_runtime_policy_state_with_runtime_rules_and_recorder(session_dir, runtime_rules, None)
}

pub(crate) fn load_runtime_policy_state_with_runtime_rules_and_recorder(
    session_dir: &Path,
    runtime_rules: Option<&capsem_proto::ipc::RuntimeSecurityRulesSnapshot>,
    match_recorder: Option<RuntimeRuleMatchAccumulator>,
) -> RuntimePolicyState {
    load_runtime_policy_state_from_effective_with_runtime_rules(
        session_dir,
        runtime_rules,
        match_recorder,
    )
}

fn load_runtime_policy_state_from_effective(session_dir: &Path) -> RuntimePolicyState {
    load_runtime_policy_state_from_effective_with_runtime_rules(session_dir, None, None)
}

fn load_runtime_policy_state_from_effective_with_runtime_rules(
    session_dir: &Path,
    runtime_rules: Option<&capsem_proto::ipc::RuntimeSecurityRulesSnapshot>,
    match_recorder: Option<RuntimeRuleMatchAccumulator>,
) -> RuntimePolicyState {
    let effective = load_effective_vm_settings_with_fallback(session_dir);

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
    let mut enforcement_rules = effective
        .as_ref()
        .map(http_runtime_enforcement_rules_from_effective)
        .unwrap_or_default();
    let mut detection_rules = Vec::new();
    if let Some(runtime_rules) = runtime_rules {
        enforcement_rules.extend(
            runtime_rules
                .enforcement
                .iter()
                .cloned()
                .map(cel_enforcement_rule_from_snapshot),
        );
        detection_rules.extend(
            runtime_rules
                .detection
                .iter()
                .cloned()
                .map(cel_detection_rule_from_snapshot),
        );
    }
    let security_engine = build_runtime_security_engine_from_rules(
        effective.as_ref(),
        enforcement_rules,
        detection_rules,
        match_recorder,
    );

    let mcp_user = effective
        .as_ref()
        .map(mcp_user_config_from_effective)
        .unwrap_or_default();
    let mcp_corp = McpUserConfig::default();
    let mcp_policy = mcp_user.to_policy(&mcp_corp);
    let guest_config = guest_config_from_effective(effective.as_ref());
    let profile_id = effective
        .as_ref()
        .map(|effective| effective.profile_id.clone())
        .unwrap_or_else(|| "unknown".to_string());

    RuntimePolicyState {
        profile_id,
        guest_config,
        domain_policy,
        security_engine,
        mcp_policy,
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

fn http_runtime_enforcement_rules_from_effective(
    effective: &settings_profiles::EffectiveVmSettings,
) -> Vec<CelEnforcementRule> {
    effective
        .rules
        .iter()
        .filter_map(http_runtime_enforcement_rule)
        .collect()
}

fn build_runtime_security_engine_from_rules(
    effective: Option<&settings_profiles::EffectiveVmSettings>,
    enforcement_rules: Vec<CelEnforcementRule>,
    detection_rules: Vec<capsem_security_engine::CelDetectionRule>,
    match_recorder: Option<RuntimeRuleMatchAccumulator>,
) -> Option<Arc<dyn RuntimeSecurityEngine>> {
    if enforcement_rules.is_empty() && detection_rules.is_empty() {
        return None;
    }

    let mut engine = SecurityEngine::default();
    if !enforcement_rules.is_empty() {
        let evaluator = match CelEnforcementEvaluator::compile(enforcement_rules) {
            Ok(evaluator) => evaluator,
            Err(error) => {
                warn!(
                    error = %error,
                    "failed to compile HTTP runtime enforcement rules; installing fail-closed network rule"
                );
                CelEnforcementEvaluator::compile(vec![CelEnforcementRule {
                    id: "runtime.compile_failed".into(),
                    pack_id: Some("runtime".into()),
                    condition: "true".into(),
                    decision: SecurityDecisionAction::Block,
                    reason: Some("HTTP runtime rules failed to compile".into()),
                }])
                .expect("static fail-closed CEL rule must compile")
            }
        };
        engine.set_enforcement(Box::new(evaluator));
    }
    if !detection_rules.is_empty() {
        match capsem_security_engine::CelDetectionEvaluator::compile(detection_rules) {
            Ok(evaluator) => engine.set_detection(Box::new(evaluator)),
            Err(error) => {
                warn!(
                    error = %error,
                    "failed to compile runtime detection rules; continuing without runtime detection"
                );
            }
        }
    }
    if let Some(match_recorder) = match_recorder {
        engine.set_match_recorder(Box::new(match_recorder));
    }
    info!(
        profile_id = %effective
            .map(|effective| effective.profile_id.as_str())
            .unwrap_or("unknown"),
        "installed HTTP runtime security engine"
    );
    let runtime: Arc<dyn RuntimeSecurityEngine> = Arc::new(Mutex::new(engine));
    Some(runtime)
}

fn cel_enforcement_rule_from_snapshot(
    rule: capsem_proto::ipc::RuntimeEnforcementRuleSnapshot,
) -> CelEnforcementRule {
    CelEnforcementRule {
        id: rule.id,
        pack_id: rule.pack_id,
        condition: rule.condition,
        decision: security_decision_action_from_snapshot(rule.decision),
        reason: rule.reason,
    }
}

fn cel_detection_rule_from_snapshot(
    rule: capsem_proto::ipc::RuntimeDetectionRuleSnapshot,
) -> capsem_security_engine::CelDetectionRule {
    capsem_security_engine::CelDetectionRule {
        id: rule.id,
        pack_id: rule.pack_id,
        sigma_id: rule.sigma_id,
        title: rule.title,
        condition: rule.condition,
        severity: severity_from_snapshot(rule.severity),
        confidence: confidence_from_snapshot(rule.confidence),
        tags: rule.tags,
    }
}

fn security_decision_action_from_snapshot(
    action: capsem_proto::ipc::RuntimeSecurityDecisionAction,
) -> SecurityDecisionAction {
    match action {
        capsem_proto::ipc::RuntimeSecurityDecisionAction::Allow => SecurityDecisionAction::Allow,
        capsem_proto::ipc::RuntimeSecurityDecisionAction::Ask => SecurityDecisionAction::Ask,
        capsem_proto::ipc::RuntimeSecurityDecisionAction::Block => SecurityDecisionAction::Block,
        capsem_proto::ipc::RuntimeSecurityDecisionAction::Rewrite => {
            SecurityDecisionAction::Rewrite
        }
        capsem_proto::ipc::RuntimeSecurityDecisionAction::Throttle => {
            SecurityDecisionAction::Throttle
        }
    }
}

fn severity_from_snapshot(
    severity: capsem_proto::ipc::RuntimeDetectionSeverity,
) -> capsem_security_engine::Severity {
    match severity {
        capsem_proto::ipc::RuntimeDetectionSeverity::Info => capsem_security_engine::Severity::Info,
        capsem_proto::ipc::RuntimeDetectionSeverity::Low => capsem_security_engine::Severity::Low,
        capsem_proto::ipc::RuntimeDetectionSeverity::Medium => {
            capsem_security_engine::Severity::Medium
        }
        capsem_proto::ipc::RuntimeDetectionSeverity::High => capsem_security_engine::Severity::High,
        capsem_proto::ipc::RuntimeDetectionSeverity::Critical => {
            capsem_security_engine::Severity::Critical
        }
    }
}

fn confidence_from_snapshot(
    confidence: capsem_proto::ipc::RuntimeDetectionConfidence,
) -> capsem_security_engine::Confidence {
    match confidence {
        capsem_proto::ipc::RuntimeDetectionConfidence::Low => {
            capsem_security_engine::Confidence::Low
        }
        capsem_proto::ipc::RuntimeDetectionConfidence::Medium => {
            capsem_security_engine::Confidence::Medium
        }
        capsem_proto::ipc::RuntimeDetectionConfidence::High => {
            capsem_security_engine::Confidence::High
        }
    }
}

fn http_runtime_enforcement_rule(rule: &EffectiveRule) -> Option<CelEnforcementRule> {
    if rule.callback != "http.request" {
        return None;
    }
    Some(CelEnforcementRule {
        id: rule.id.clone(),
        pack_id: Some(rule.provenance.profile_id.clone()),
        condition: rule.condition.clone(),
        decision: profile_decision_to_security_action(rule.decision),
        reason: rule.reason.clone(),
    })
}

fn profile_decision_to_security_action(decision: RuleDecision) -> SecurityDecisionAction {
    match decision {
        RuleDecision::Allow => SecurityDecisionAction::Allow,
        RuleDecision::Ask => SecurityDecisionAction::Ask,
        RuleDecision::Block => SecurityDecisionAction::Block,
        RuleDecision::Rewrite => SecurityDecisionAction::Rewrite,
    }
}

fn domain_from_simple_network_condition(rule: &EffectiveRule) -> Option<String> {
    match rule.callback.as_str() {
        "dns.request" => extract_condition_eq(&rule.condition, "qname"),
        "http.request" | "http.read" | "http.write" | "http.response" => {
            extract_condition_eq(&rule.condition, "http.request.host")
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

    let servers = effective
        .mcp
        .value
        .connectors
        .iter()
        .map(|(id, connector)| McpManualServer {
            name: id.clone(),
            url: connector.url.clone().unwrap_or_default(),
            command: connector.command.clone(),
            args: connector.args.clone(),
            env: connector
                .env
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            headers: connector
                .headers
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            bearer_token: connector.bearer_token.clone(),
            pool_size: connector.pool_size,
            pool_safe_tools: connector.pool_safe_tools.clone(),
            enabled: connector.enabled,
        })
        .collect::<Vec<_>>();

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
        servers,
        server_enabled,
        tool_permissions,
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
