use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::sync::{Mutex, OnceLock};

use capsem_core::mcp::policy::ToolDecision;
use capsem_core::settings_profiles::{
    CapabilityMode, EffectiveRule, McpConnectorCapsemMetadata, McpConnectorConfig, RuleDecision,
};
use capsem_network_engine::domain_policy::{Action, DomainPolicy};
use capsem_security_engine::{
    AiAttributionScope, AiOriginKind, Enforceability, HttpSecuritySubject, ProcessSecuritySubject,
    RedactionState, SecurityAction, SecurityEvent, SecurityEventCommon, SourceEngine,
};

use capsem_core::mcp::policy::McpUserConfig;

use super::{
    build_builtin_env, build_servers_with_builtin, insert_builtin_domain_policy_env,
    load_runtime_policy_state, load_runtime_policy_state_from_effective,
    load_runtime_policy_state_with_runtime_rules,
    load_runtime_policy_state_with_runtime_rules_and_recorder, RuntimeRuleMatchAccumulator,
};

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct EnvGuard {
    key: &'static str,
    old: Option<OsString>,
}

impl EnvGuard {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let old = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, old }
    }

    fn remove(key: &'static str) -> Self {
        let old = std::env::var_os(key);
        std::env::remove_var(key);
        Self { key, old }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        if let Some(old) = &self.old {
            std::env::set_var(self.key, old);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

#[test]
fn builtin_domain_policy_env_carries_allow_and_block_lists() {
    let policy = DomainPolicy::new(
        &["example.com".to_string(), "*.trusted.test".to_string()],
        &["blocked.test".to_string()],
        Action::Deny,
    );
    let mut env = HashMap::new();

    insert_builtin_domain_policy_env(&mut env, &policy);

    assert_eq!(
        env.get("CAPSEM_DOMAIN_ALLOW").map(String::as_str),
        Some("example.com,*.trusted.test")
    );
    assert_eq!(
        env.get("CAPSEM_DOMAIN_BLOCK").map(String::as_str),
        Some("blocked.test")
    );
    assert_eq!(
        env.get("CAPSEM_DOMAIN_DEFAULT").map(String::as_str),
        Some("deny")
    );
}

#[test]
fn builtin_domain_policy_env_leaves_open_policy_lists_unset() {
    let policy = DomainPolicy::new(&[], &[], Action::Allow);
    let mut env = HashMap::new();

    insert_builtin_domain_policy_env(&mut env, &policy);

    assert!(!env.contains_key("CAPSEM_DOMAIN_ALLOW"));
    assert!(!env.contains_key("CAPSEM_DOMAIN_BLOCK"));
    assert_eq!(
        env.get("CAPSEM_DOMAIN_DEFAULT").map(String::as_str),
        Some("allow")
    );
}

#[test]
fn build_builtin_env_includes_session_paths_and_domain_policy() {
    let policy = DomainPolicy::new(
        &["example.com".to_string()],
        &["blocked.test".to_string()],
        Action::Deny,
    );

    let env = build_builtin_env(std::path::Path::new("/tmp/capsem/session"), &policy);

    assert_eq!(
        env.get("CAPSEM_SESSION_DIR").map(String::as_str),
        Some("/tmp/capsem/session")
    );
    assert_eq!(
        env.get("CAPSEM_SESSION_DB").map(String::as_str),
        Some("/tmp/capsem/session/session.db")
    );
    assert_eq!(
        env.get("CAPSEM_DOMAIN_ALLOW").map(String::as_str),
        Some("example.com")
    );
    assert_eq!(
        env.get("CAPSEM_DOMAIN_BLOCK").map(String::as_str),
        Some("blocked.test")
    );
    assert_eq!(
        env.get("CAPSEM_DOMAIN_DEFAULT").map(String::as_str),
        Some("deny")
    );
}

#[test]
fn build_servers_with_builtin_preserves_local_session_and_domain_env() {
    let dir = tempfile::tempdir().unwrap();
    let builtin = dir.path().join("capsem-mcp-builtin");
    std::fs::write(&builtin, b"fake").unwrap();
    let session = dir.path().join("session");
    let policy = DomainPolicy::new(
        &["example.com".to_string()],
        &["blocked.test".to_string()],
        Action::Deny,
    );

    let servers = build_servers_with_builtin(
        &McpUserConfig::default(),
        &McpUserConfig::default(),
        Some(&builtin),
        &session,
        &policy,
    );

    let local = servers
        .iter()
        .find(|server| server.name == "local")
        .expect("local builtin server should be present");
    assert_eq!(local.command.as_deref(), Some(builtin.to_str().unwrap()));
    assert_eq!(
        local.env.get("CAPSEM_SESSION_DIR").map(String::as_str),
        Some(session.to_str().unwrap())
    );
    assert_eq!(
        local.env.get("CAPSEM_SESSION_DB").map(String::as_str),
        Some(session.join("session.db").to_str().unwrap())
    );
    assert_eq!(
        local.env.get("CAPSEM_DOMAIN_ALLOW").map(String::as_str),
        Some("example.com")
    );
    assert_eq!(
        local.env.get("CAPSEM_DOMAIN_BLOCK").map(String::as_str),
        Some("blocked.test")
    );
    assert_eq!(
        local.env.get("CAPSEM_DOMAIN_DEFAULT").map(String::as_str),
        Some("deny")
    );
}

#[test]
fn load_runtime_policy_state_converts_vm_effective_rules_and_mcp_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("session");
    std::fs::create_dir_all(&session_dir).unwrap();

    let roots = capsem_core::settings_profiles::ProfileRootSettings::default();
    let mut effective = capsem_core::settings_profiles::resolve_effective_vm_settings(&roots, None)
        .expect("default effective profile should resolve");
    effective.security.value.capabilities.network_egress = CapabilityMode::Block;
    effective.security.value.capabilities.mcp_tools = CapabilityMode::Ask;
    let provenance = effective.profile.provenance.clone();

    effective.rules.push(EffectiveRule {
        id: "mcp.block-prod-delete".to_string(),
        callback: "mcp.request".to_string(),
        condition: "method == \"tools/call\" && tool.name == \"github__delete_repo\"".to_string(),
        decision: RuleDecision::Block,
        priority: 1,
        rewrite_target: None,
        rewrite_value: None,
        strip_request_headers: Vec::new(),
        strip_response_headers: Vec::new(),
        reason: Some("Block delete repo".to_string()),
        derived: false,
        provenance: provenance.clone(),
        owner_setting_path: None,
        owner_setting_label: None,
        editable: true,
    });
    effective.rules.push(EffectiveRule {
        id: "mcp.block-any-dangerous-tool".to_string(),
        callback: "mcp.request".to_string(),
        condition: "tool.name == \"danger__run\"".to_string(),
        decision: RuleDecision::Block,
        priority: 1,
        rewrite_target: None,
        rewrite_value: None,
        strip_request_headers: Vec::new(),
        strip_response_headers: Vec::new(),
        reason: Some("Block dangerous tool".to_string()),
        derived: false,
        provenance: provenance.clone(),
        owner_setting_path: None,
        owner_setting_label: None,
        editable: true,
    });
    effective.rules.push(EffectiveRule {
        id: "http.block-secret-content".to_string(),
        callback: "http.response".to_string(),
        condition: "response.text.contains(\"secret\")".to_string(),
        decision: RuleDecision::Block,
        priority: 1,
        rewrite_target: None,
        rewrite_value: None,
        strip_request_headers: Vec::new(),
        strip_response_headers: Vec::new(),
        reason: Some("Block leaked secret".to_string()),
        derived: false,
        provenance,
        owner_setting_path: None,
        owner_setting_label: None,
        editable: true,
    });
    effective.rules.push(EffectiveRule {
        id: "http.allow-example-domain".to_string(),
        callback: "http.request".to_string(),
        condition: "http.request.host == \"example.com\"".to_string(),
        decision: RuleDecision::Allow,
        priority: 900,
        rewrite_target: None,
        rewrite_value: None,
        strip_request_headers: Vec::new(),
        strip_response_headers: Vec::new(),
        reason: Some("Allow example.com".to_string()),
        derived: false,
        provenance: effective.profile.provenance.clone(),
        owner_setting_path: None,
        owner_setting_label: None,
        editable: true,
    });
    effective.rules.push(EffectiveRule {
        id: "http.block-example-secret-path".to_string(),
        callback: "http.request".to_string(),
        condition: "http.request.host == \"example.com\" && http.request.path == \"/secret\""
            .to_string(),
        decision: RuleDecision::Block,
        priority: 10,
        rewrite_target: None,
        rewrite_value: None,
        strip_request_headers: Vec::new(),
        strip_response_headers: Vec::new(),
        reason: Some("Block one path only".to_string()),
        derived: false,
        provenance: effective.profile.provenance.clone(),
        owner_setting_path: None,
        owner_setting_label: None,
        editable: true,
    });
    effective.rules.push(EffectiveRule {
        id: "http.block-bad-domain".to_string(),
        callback: "http.request".to_string(),
        condition: "http.request.host == \"bad.example\"".to_string(),
        decision: RuleDecision::Block,
        priority: 10,
        rewrite_target: None,
        rewrite_value: None,
        strip_request_headers: Vec::new(),
        strip_response_headers: Vec::new(),
        reason: Some("Block bad.example".to_string()),
        derived: false,
        provenance: effective.profile.provenance.clone(),
        owner_setting_path: None,
        owner_setting_label: None,
        editable: true,
    });
    effective.rules.push(EffectiveRule {
        id: "dns.block-bad-domain".to_string(),
        callback: "dns.request".to_string(),
        condition: "dns.request.qname == \"blocked-dns.example\"".to_string(),
        decision: RuleDecision::Block,
        priority: 10,
        rewrite_target: None,
        rewrite_value: None,
        strip_request_headers: Vec::new(),
        strip_response_headers: Vec::new(),
        reason: Some("Block blocked-dns.example".to_string()),
        derived: false,
        provenance: effective.profile.provenance.clone(),
        owner_setting_path: None,
        owner_setting_label: None,
        editable: true,
    });
    effective.rules.push(EffectiveRule {
        id: "dns.rewrite-fixture".to_string(),
        callback: "dns.request".to_string(),
        condition: "dns.request.qname == \"rewrite-dns.example\"".to_string(),
        decision: RuleDecision::Rewrite,
        priority: 11,
        rewrite_target: Some("answer.ip =~ \".*\"".to_string()),
        rewrite_value: Some("203.0.113.77".to_string()),
        strip_request_headers: Vec::new(),
        strip_response_headers: Vec::new(),
        reason: Some("Rewrite DNS answer".to_string()),
        derived: false,
        provenance: effective.profile.provenance.clone(),
        owner_setting_path: None,
        owner_setting_label: None,
        editable: true,
    });
    effective.rules.push(EffectiveRule {
        id: "http.user-read".to_string(),
        callback: "http.read".to_string(),
        condition: "true".to_string(),
        decision: RuleDecision::Ask,
        priority: 20,
        rewrite_target: None,
        rewrite_value: None,
        strip_request_headers: Vec::new(),
        strip_response_headers: Vec::new(),
        reason: Some("User-authored read gate".to_string()),
        derived: false,
        provenance: effective.profile.provenance.clone(),
        owner_setting_path: None,
        owner_setting_label: None,
        editable: true,
    });
    effective.rules.push(EffectiveRule {
        id: "http.user-write".to_string(),
        callback: "http.write".to_string(),
        condition: "true".to_string(),
        decision: RuleDecision::Block,
        priority: 21,
        rewrite_target: None,
        rewrite_value: None,
        strip_request_headers: Vec::new(),
        strip_response_headers: Vec::new(),
        reason: Some("User-authored write gate".to_string()),
        derived: false,
        provenance: effective.profile.provenance.clone(),
        owner_setting_path: None,
        owner_setting_label: None,
        editable: true,
    });

    capsem_core::settings_profiles::write_vm_effective_settings(&session_dir, &effective).unwrap();

    let runtime = load_runtime_policy_state_from_effective(&session_dir);

    assert_eq!(runtime.domain_policy.default_action(), Action::Deny);
    assert_eq!(runtime.mcp_policy.default_tool_decision, ToolDecision::Warn);
    assert!(
        !runtime
            .mcp_policy
            .tool_decisions
            .contains_key("github__delete_repo"),
        "conditional Profile V2 rules must stay in the exact policy engine"
    );
    assert_eq!(
        runtime
            .mcp_policy
            .tool_decisions
            .get("danger__run")
            .copied(),
        Some(ToolDecision::Block)
    );
    assert!(runtime
        .domain_policy
        .allowed_patterns()
        .contains(&"example.com".to_string()));
    assert_eq!(
        runtime.domain_policy.blocked_patterns(),
        vec!["bad.example".to_string(), "blocked-dns.example".to_string()]
    );
    assert_eq!(
        runtime.domain_policy.evaluate("example.com").0,
        Action::Allow
    );
    assert_eq!(
        runtime.domain_policy.evaluate("bad.example").0,
        Action::Deny
    );
    assert!(
        runtime
            .domain_policy
            .blocked_patterns()
            .contains(&"bad.example".to_string()),
        "simple domain block rules must feed DNS-level full-block policy"
    );
    assert!(
        runtime
            .domain_policy
            .blocked_patterns()
            .contains(&"blocked-dns.example".to_string()),
        "simple DNS block rules must feed DNS-level full-block policy"
    );
    assert!(
        !runtime
            .domain_policy
            .blocked_patterns()
            .contains(&"example.com".to_string()),
        "path-scoped HTTP blocks must not become full-domain DNS blocks"
    );

    let security_engine = runtime
        .security_engine
        .as_ref()
        .expect("canonical HTTP rules should install a runtime Security Engine");
    let blocked = security_engine
        .evaluate(http_event("bad.example", "/"))
        .expect("profile runtime engine should evaluate canonical HTTP CEL");
    assert!(matches!(blocked.action, SecurityAction::Block(_)));
    assert_eq!(
        blocked
            .resolved_event
            .event
            .decision
            .as_ref()
            .and_then(|decision| decision.rule.as_deref()),
        Some("policy.http.block-bad-domain")
    );
    let dns_blocked = security_engine
        .evaluate(
            capsem_network_engine::dns_security::build_dns_security_event_from_query(
                &capsem_network_engine::dns_parser::DnsQuery {
                    id: 7,
                    qname: "blocked-dns.example".into(),
                    qtype: 1,
                    qclass: 1,
                    extra_questions: 0,
                },
                None,
            ),
        )
        .expect("profile runtime engine should evaluate canonical DNS CEL");
    assert!(matches!(dns_blocked.action, SecurityAction::Block(_)));
    assert_eq!(
        dns_blocked
            .resolved_event
            .event
            .decision
            .as_ref()
            .and_then(|decision| decision.rule.as_deref()),
        Some("policy.dns.block-bad-domain")
    );
    let dns_rewritten = security_engine
        .evaluate(
            capsem_network_engine::dns_security::build_dns_security_event_from_query(
                &capsem_network_engine::dns_parser::DnsQuery {
                    id: 8,
                    qname: "rewrite-dns.example".into(),
                    qtype: 1,
                    qclass: 1,
                    extra_questions: 0,
                },
                None,
            ),
        )
        .expect("profile runtime engine should evaluate canonical DNS rewrite CEL");
    assert!(matches!(dns_rewritten.action, SecurityAction::Rewrite(_)));
    assert_eq!(
        dns_rewritten
            .resolved_event
            .event
            .decision
            .as_ref()
            .and_then(|decision| decision.rule.as_deref()),
        Some("policy.dns.rewrite-fixture")
    );
    assert_eq!(dns_rewritten.resolved_event.event.mutations.len(), 1);
}

#[test]
fn default_profile_runtime_engine_allows_reads_and_writes_while_ask_is_deferred() {
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("session");
    std::fs::create_dir_all(&session_dir).unwrap();

    let roots = capsem_core::settings_profiles::ProfileRootSettings::default();
    let effective = capsem_core::settings_profiles::resolve_effective_vm_settings(&roots, None)
        .expect("default effective profile should resolve");
    capsem_core::settings_profiles::write_vm_effective_settings(&session_dir, &effective).unwrap();

    let runtime = load_runtime_policy_state_from_effective(&session_dir);
    let security_engine = runtime
        .security_engine
        .as_ref()
        .expect("default read/write rules should install a runtime Security Engine");

    let read = security_engine
        .evaluate(http_event_with_method("GET", "example.com", "/"))
        .expect("default HTTP read rule should evaluate");
    assert!(
        matches!(read.action, SecurityAction::Continue),
        "default Profile V2 should allow HTTP reads until a stronger rule matches"
    );

    let write = security_engine
        .evaluate(http_event_with_method("POST", "example.com", "/"))
        .expect("default HTTP write rule should evaluate");
    assert!(
        matches!(write.action, SecurityAction::Continue),
        "default Profile V2 network_egress=ask resolves as allow until S15 wires a confirm resolver"
    );
    assert_eq!(
        write
            .resolved_event
            .event
            .decision
            .as_ref()
            .and_then(|decision| decision.rule.as_deref()),
        Some("http.default_write")
    );
}

#[test]
fn load_runtime_policy_state_merges_service_runtime_rule_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("session");
    std::fs::create_dir_all(&session_dir).unwrap();

    let roots = capsem_core::settings_profiles::ProfileRootSettings::default();
    let effective = capsem_core::settings_profiles::resolve_effective_vm_settings(&roots, None)
        .expect("default effective profile should resolve");
    capsem_core::settings_profiles::write_vm_effective_settings(&session_dir, &effective).unwrap();
    let snapshot = capsem_proto::ipc::RuntimeSecurityRulesSnapshot {
        enforcement: vec![capsem_proto::ipc::RuntimeEnforcementRuleSnapshot {
            id: "runtime.block-live".into(),
            pack_id: Some("runtime-pack".into()),
            condition: "http.request.host == 'live-policy.test'".into(),
            decision: capsem_proto::ipc::RuntimeSecurityDecisionAction::Block,
            reason: Some("live runtime block".into()),
        }, capsem_proto::ipc::RuntimeEnforcementRuleSnapshot {
            id: "runtime.block-process-shell".into(),
            pack_id: Some("runtime-pack".into()),
            condition: "process.activity.operation == 'exec' && process.activity.command_class == 'shell'".into(),
            decision: capsem_proto::ipc::RuntimeSecurityDecisionAction::Block,
            reason: Some("shell exec block".into()),
        }],
        detection: vec![capsem_proto::ipc::RuntimeDetectionRuleSnapshot {
            id: "runtime.detect-live".into(),
            pack_id: "runtime-detection".into(),
            sigma_id: Some("sigma-live".into()),
            title: "Live runtime detection".into(),
            condition: "http.request.host == 'observe-policy.test'".into(),
            severity: capsem_proto::ipc::RuntimeDetectionSeverity::High,
            confidence: capsem_proto::ipc::RuntimeDetectionConfidence::High,
            tags: vec!["runtime".into()],
        }, capsem_proto::ipc::RuntimeDetectionRuleSnapshot {
            id: "runtime.detect-process-python".into(),
            pack_id: "runtime-detection".into(),
            sigma_id: Some("sigma-process".into()),
            title: "Python exec detection".into(),
            condition: "process.activity.operation == 'exec' && process.activity.command_class == 'python'".into(),
            severity: capsem_proto::ipc::RuntimeDetectionSeverity::Medium,
            confidence: capsem_proto::ipc::RuntimeDetectionConfidence::High,
            tags: vec!["process".into()],
        }],
    };

    let runtime = load_runtime_policy_state_with_runtime_rules(&session_dir, Some(&snapshot));
    let security_engine = runtime
        .security_engine
        .as_ref()
        .expect("runtime rule snapshot should install a Security Engine");

    let blocked = security_engine
        .evaluate(http_event("live-policy.test", "/"))
        .expect("runtime snapshot enforcement should evaluate");
    assert!(matches!(blocked.action, SecurityAction::Block(_)));
    assert_eq!(
        blocked
            .resolved_event
            .event
            .decision
            .as_ref()
            .and_then(|decision| decision.rule.as_deref()),
        Some("runtime.block-live")
    );

    let detected = security_engine
        .evaluate(http_event("observe-policy.test", "/"))
        .expect("runtime snapshot detection should evaluate");
    assert!(matches!(detected.action, SecurityAction::Continue));
    assert_eq!(detected.resolved_event.event.findings.len(), 1);
    assert_eq!(
        detected.resolved_event.event.findings[0].rule_id,
        "runtime.detect-live"
    );

    let blocked_process = security_engine
        .evaluate(process_event("exec-shell", "exec", Some("shell")))
        .expect("runtime snapshot process enforcement should evaluate");
    assert!(matches!(blocked_process.action, SecurityAction::Block(_)));
    assert_eq!(
        blocked_process
            .resolved_event
            .event
            .decision
            .as_ref()
            .and_then(|decision| decision.rule.as_deref()),
        Some("runtime.block-process-shell")
    );

    let detected_process = security_engine
        .evaluate(process_event("exec-python", "exec", Some("python")))
        .expect("runtime snapshot process detection should evaluate");
    assert!(matches!(detected_process.action, SecurityAction::Continue));
    assert_eq!(
        detected_process.resolved_event.event.findings[0].rule_id,
        "runtime.detect-process-python"
    );
}

#[test]
fn runtime_rule_match_accumulator_drains_recorded_security_engine_matches() {
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("session");
    std::fs::create_dir_all(&session_dir).unwrap();

    let roots = capsem_core::settings_profiles::ProfileRootSettings::default();
    let effective = capsem_core::settings_profiles::resolve_effective_vm_settings(&roots, None)
        .expect("default effective profile should resolve");
    capsem_core::settings_profiles::write_vm_effective_settings(&session_dir, &effective).unwrap();
    let snapshot = capsem_proto::ipc::RuntimeSecurityRulesSnapshot {
        enforcement: vec![
            capsem_proto::ipc::RuntimeEnforcementRuleSnapshot {
                id: "runtime.block-live".into(),
                pack_id: Some("runtime-pack".into()),
                condition: "http.request.host == 'live-policy.test'".into(),
                decision: capsem_proto::ipc::RuntimeSecurityDecisionAction::Block,
                reason: Some("live runtime block".into()),
            },
            capsem_proto::ipc::RuntimeEnforcementRuleSnapshot {
                id: "runtime.block-process-shell".into(),
                pack_id: Some("runtime-pack".into()),
                condition:
                    "process.activity.operation == 'exec' && process.activity.command_class == 'shell'"
                        .into(),
                decision: capsem_proto::ipc::RuntimeSecurityDecisionAction::Block,
                reason: Some("shell exec block".into()),
            },
        ],
        detection: vec![capsem_proto::ipc::RuntimeDetectionRuleSnapshot {
            id: "runtime.detect-process-python".into(),
            pack_id: "runtime-detection".into(),
            sigma_id: Some("sigma-process".into()),
            title: "Python exec detection".into(),
            condition:
                "process.activity.operation == 'exec' && process.activity.command_class == 'python'"
                    .into(),
            severity: capsem_proto::ipc::RuntimeDetectionSeverity::Medium,
            confidence: capsem_proto::ipc::RuntimeDetectionConfidence::High,
            tags: vec!["process".into()],
        }],
    };
    let accumulator = RuntimeRuleMatchAccumulator::default();
    let runtime = load_runtime_policy_state_with_runtime_rules_and_recorder(
        &session_dir,
        Some(&snapshot),
        Some(accumulator.clone()),
    );
    let security_engine = runtime
        .security_engine
        .as_ref()
        .expect("runtime rule snapshot should install a Security Engine");

    security_engine
        .evaluate(http_event("live-policy.test", "/first"))
        .expect("first rule match should evaluate");
    security_engine
        .evaluate(http_event("live-policy.test", "/second"))
        .expect("second rule match should evaluate");
    security_engine
        .evaluate(process_event("exec-shell", "exec", Some("shell")))
        .expect("process enforcement match should evaluate");
    security_engine
        .evaluate(process_event("exec-python", "exec", Some("python")))
        .expect("process detection match should evaluate");

    let drained = accumulator
        .drain()
        .into_iter()
        .map(|rule_match| (rule_match.rule_id.clone(), rule_match))
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(drained.len(), 3);
    let http = drained.get("runtime.block-live").unwrap();
    assert_eq!(http.match_count, 2);
    assert_eq!(
        http.last_matched_event.as_deref(),
        Some("test-http-GET-live-policy.test-/second")
    );
    let shell = drained.get("runtime.block-process-shell").unwrap();
    assert_eq!(shell.match_count, 1);
    assert_eq!(shell.last_matched_event.as_deref(), Some("exec-shell"));
    let python = drained.get("runtime.detect-process-python").unwrap();
    assert_eq!(python.match_count, 1);
    assert_eq!(python.last_matched_event.as_deref(), Some("exec-python"));
    assert!(
        accumulator.drain().is_empty(),
        "drain must return deltas, not replay old matches"
    );
}

#[test]
fn pooled_runtime_security_engine_records_parallel_rule_matches() {
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("session");
    std::fs::create_dir_all(&session_dir).unwrap();

    let roots = capsem_core::settings_profiles::ProfileRootSettings::default();
    let effective = capsem_core::settings_profiles::resolve_effective_vm_settings(&roots, None)
        .expect("default effective profile should resolve");
    capsem_core::settings_profiles::write_vm_effective_settings(&session_dir, &effective).unwrap();
    let snapshot = capsem_proto::ipc::RuntimeSecurityRulesSnapshot {
        enforcement: vec![capsem_proto::ipc::RuntimeEnforcementRuleSnapshot {
            id: "runtime.block-live".into(),
            pack_id: Some("runtime-pack".into()),
            condition: "http.request.host == 'live-policy.test'".into(),
            decision: capsem_proto::ipc::RuntimeSecurityDecisionAction::Block,
            reason: Some("live runtime block".into()),
        }],
        detection: Vec::new(),
    };
    let accumulator = RuntimeRuleMatchAccumulator::default();
    let runtime = load_runtime_policy_state_with_runtime_rules_and_recorder(
        &session_dir,
        Some(&snapshot),
        Some(accumulator.clone()),
    );
    let security_engine = runtime
        .security_engine
        .as_ref()
        .expect("runtime rule snapshot should install a Security Engine")
        .clone();

    let handles = (0..16)
        .map(|index| {
            let security_engine = security_engine.clone();
            std::thread::spawn(move || {
                let result = security_engine
                    .evaluate(http_event(
                        "live-policy.test",
                        &format!("/parallel-{index}"),
                    ))
                    .expect("parallel runtime security evaluation should succeed");
                assert!(matches!(result.action, SecurityAction::Block(_)));
            })
        })
        .collect::<Vec<_>>();
    for handle in handles {
        handle.join().unwrap();
    }

    let drained = accumulator
        .drain()
        .into_iter()
        .map(|rule_match| (rule_match.rule_id.clone(), rule_match))
        .collect::<std::collections::BTreeMap<_, _>>();
    let matched = drained.get("runtime.block-live").unwrap();
    assert_eq!(matched.match_count, 16);
    assert!(matched
        .last_matched_event
        .as_deref()
        .is_some_and(|event_id| event_id.starts_with("test-http-GET-live-policy.test-")));
}

#[test]
fn invalid_runtime_process_rule_fails_closed_with_generic_reason() {
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("session");
    std::fs::create_dir_all(&session_dir).unwrap();

    let roots = capsem_core::settings_profiles::ProfileRootSettings::default();
    let effective = capsem_core::settings_profiles::resolve_effective_vm_settings(&roots, None)
        .expect("default effective profile should resolve");
    capsem_core::settings_profiles::write_vm_effective_settings(&session_dir, &effective).unwrap();
    let snapshot = capsem_proto::ipc::RuntimeSecurityRulesSnapshot {
        enforcement: vec![capsem_proto::ipc::RuntimeEnforcementRuleSnapshot {
            id: "runtime.bad-process-rule".into(),
            pack_id: Some("runtime-pack".into()),
            condition: "process.activity.command_class ==".into(),
            decision: capsem_proto::ipc::RuntimeSecurityDecisionAction::Block,
            reason: Some("bad process rule".into()),
        }],
        detection: vec![],
    };

    let runtime = load_runtime_policy_state_with_runtime_rules(&session_dir, Some(&snapshot));
    let security_engine = runtime
        .security_engine
        .as_ref()
        .expect("compile failure should still install a fail-closed Security Engine");

    let result = security_engine
        .evaluate(process_event("exec-after-bad-rule", "exec", Some("shell")))
        .expect("fail-closed process rule should evaluate");

    match result.action {
        SecurityAction::Block(block) => {
            assert_eq!(block.rule_id.as_deref(), Some("runtime.compile_failed"));
            assert_eq!(
                block.reason_code,
                "runtime security rules failed to compile"
            );
        }
        other => panic!("expected fail-closed process block, got {other:?}"),
    }
}

fn http_event(host: &str, path: &str) -> SecurityEvent {
    http_event_with_method("GET", host, path)
}

fn http_event_with_method(method: &str, host: &str, path: &str) -> SecurityEvent {
    SecurityEvent::http(
        SecurityEventCommon {
            event_id: format!("test-http-{method}-{host}-{path}"),
            parent_event_id: None,
            stream_id: None,
            activity_id: None,
            sequence_no: None,
            source_engine: SourceEngine::Network,
            attribution_scope: AiAttributionScope::Vm,
            origin_kind: AiOriginKind::GuestNetwork,
            accounting_owner: None,
            enforceability: Enforceability::InlineBlockable,
            trace_id: Some("trace-test".into()),
            span_id: None,
            timestamp_unix_ms: 1,
            vm_id: None,
            session_id: None,
            profile_id: None,
            profile_revision: None,
            profile_pack_ids: Vec::new(),
            enforcement_packs: Vec::new(),
            detection_packs: Vec::new(),
            user_id: None,
            process_id: None,
            parent_process_id: None,
            exec_id: None,
            turn_id: None,
            message_id: None,
            tool_call_id: None,
            mcp_call_id: None,
            event_type: "http.request".into(),
            redaction_state: RedactionState::Raw,
        },
        HttpSecuritySubject {
            method: method.into(),
            scheme: Some("https".into()),
            host: host.into(),
            port: Some(443),
            path: Some(path.into()),
            url: Some(format!("https://{host}{path}")),
            path_class: path.trim_start_matches('/').to_string(),
            ..HttpSecuritySubject::default()
        },
    )
}

fn process_event(event_id: &str, operation: &str, command_class: Option<&str>) -> SecurityEvent {
    SecurityEvent::process(
        SecurityEventCommon {
            event_id: event_id.into(),
            parent_event_id: None,
            stream_id: None,
            activity_id: None,
            sequence_no: None,
            source_engine: SourceEngine::Process,
            attribution_scope: AiAttributionScope::Vm,
            origin_kind: AiOriginKind::HostService,
            accounting_owner: None,
            enforceability: Enforceability::InlineBlockable,
            trace_id: Some("trace-process-test".into()),
            span_id: None,
            timestamp_unix_ms: 1,
            vm_id: None,
            session_id: None,
            profile_id: None,
            profile_revision: None,
            profile_pack_ids: Vec::new(),
            enforcement_packs: Vec::new(),
            detection_packs: Vec::new(),
            user_id: None,
            process_id: None,
            parent_process_id: None,
            exec_id: None,
            turn_id: None,
            message_id: None,
            tool_call_id: None,
            mcp_call_id: None,
            event_type: "process.exec".into(),
            redaction_state: RedactionState::Raw,
        },
        ProcessSecuritySubject {
            operation: operation.into(),
            command_class: command_class.map(str::to_owned),
        },
    )
}

#[test]
fn load_runtime_policy_state_wires_profile_mcp_servers_into_runtime_config() {
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("session");
    std::fs::create_dir_all(&session_dir).unwrap();

    let roots = capsem_core::settings_profiles::ProfileRootSettings::default();
    let mut effective =
        capsem_core::settings_profiles::resolve_effective_vm_settings(&roots, None).unwrap();
    effective.mcp.value.connectors.insert(
        "github".to_string(),
        McpConnectorConfig {
            enabled: true,
            server_type: Some("stdio".to_string()),
            command: Some("npx".to_string()),
            args: vec![
                "-y".to_string(),
                "@modelcontextprotocol/server-github".to_string(),
            ],
            env: BTreeMap::from([(
                "GITHUB_TOKEN".to_string(),
                "env:CAPSEM_GITHUB_TOKEN".to_string(),
            )]),
            url: None,
            headers: BTreeMap::new(),
            bearer_token: None,
            pool_size: Some(2),
            pool_safe_tools: vec!["repo.read".to_string()],
            capsem: McpConnectorCapsemMetadata {
                allowed_tools: vec!["repo.read".to_string()],
                ..Default::default()
            },
        },
    );

    capsem_core::settings_profiles::write_vm_effective_settings(&session_dir, &effective).unwrap();

    let runtime = load_runtime_policy_state_from_effective(&session_dir);

    let github = runtime
        .mcp_user
        .servers
        .iter()
        .find(|server| server.name == "github")
        .expect("profile mcpServers.github should become runtime MCP server");
    assert_eq!(github.command.as_deref(), Some("npx"));
    assert_eq!(
        github.args,
        vec![
            "-y".to_string(),
            "@modelcontextprotocol/server-github".to_string()
        ]
    );
    assert_eq!(
        github.env.get("GITHUB_TOKEN").map(String::as_str),
        Some("env:CAPSEM_GITHUB_TOKEN")
    );
    assert_eq!(github.pool_size, Some(2));
    assert_eq!(github.pool_safe_tools, vec!["repo.read".to_string()]);
    assert!(github.enabled);
}

#[test]
fn load_runtime_policy_state_ignores_global_legacy_user_toml() {
    let _lock = env_lock().lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let capsem_home = dir.path().join("capsem-home");
    std::fs::create_dir_all(&capsem_home).unwrap();
    let _home = EnvGuard::set("CAPSEM_HOME", &capsem_home);
    let _user = EnvGuard::remove("CAPSEM_USER_CONFIG");
    let _corp = EnvGuard::remove("CAPSEM_CORP_CONFIG");

    std::fs::write(
        capsem_home.join("user.toml"),
        r#"
[settings]
"security.web.allow_read" = { value = true, modified = "2026-05-17T00:00:00Z" }
"security.web.allow_write" = { value = true, modified = "2026-05-17T00:00:00Z" }
"security.web.custom_allow" = { value = "legacy-only.test", modified = "2026-05-17T00:00:00Z" }
"#,
    )
    .unwrap();

    let session_dir = dir.path().join("session");
    std::fs::create_dir_all(&session_dir).unwrap();
    let roots = capsem_core::settings_profiles::ProfileRootSettings::default();
    let mut effective =
        capsem_core::settings_profiles::resolve_effective_vm_settings(&roots, None).unwrap();
    effective.security.value.capabilities.network_egress = CapabilityMode::Block;
    effective.rules.clear();
    capsem_core::settings_profiles::write_vm_effective_settings(&session_dir, &effective).unwrap();

    let runtime = load_runtime_policy_state(&session_dir);

    assert!(
        runtime.domain_policy.default_action() == Action::Deny,
        "network_egress=block must win over legacy network allow defaults"
    );
    assert!(
        !runtime
            .domain_policy
            .allowed_patterns()
            .contains(&"legacy-only.test".to_string()),
        "global user.toml custom_allow must not leak into Profile V2 runtime"
    );
}

#[test]
fn load_runtime_policy_state_builds_guest_boot_contract_from_v2_effective_settings() {
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("session");
    std::fs::create_dir_all(&session_dir).unwrap();

    let roots = capsem_core::settings_profiles::ProfileRootSettings::default();
    let mut effective = capsem_core::settings_profiles::resolve_effective_vm_settings(&roots, None)
        .expect("default effective profile should resolve");
    effective
        .credential_env
        .insert("GEMINI_API_KEY".to_string(), "gemini-test-key".to_string());
    capsem_core::settings_profiles::write_vm_effective_settings(&session_dir, &effective).unwrap();
    let reloaded =
        capsem_core::settings_profiles::load_vm_effective_settings(&session_dir).unwrap();
    assert_eq!(
        reloaded
            .credential_env
            .get("GEMINI_API_KEY")
            .map(String::as_str),
        Some("gemini-test-key")
    );

    let runtime = load_runtime_policy_state_from_effective(&session_dir);
    let env = runtime
        .guest_config
        .env
        .as_ref()
        .expect("Profile V2 guest env should be built without legacy settings");
    assert_eq!(
        env.get("SSL_CERT_FILE").map(String::as_str),
        Some("/etc/ssl/certs/ca-certificates.crt")
    );
    assert_eq!(
        env.get("CAPSEM_WEB_ALLOW_READ").map(String::as_str),
        Some("1")
    );
    assert_eq!(
        env.get("CAPSEM_WEB_ALLOW_WRITE").map(String::as_str),
        Some("1")
    );
    assert_eq!(env.get("TERM").map(String::as_str), Some("xterm-256color"));
    assert_eq!(env.get("LANG").map(String::as_str), Some("C"));
    assert!(
        env.get("PATH")
            .map(|path| path.split(':').any(|entry| entry == "/opt/ai-clis/bin"))
            .unwrap_or(false),
        "PATH must include /opt/ai-clis/bin for npm-installed AI CLIs"
    );
    assert_eq!(
        env.get("VIRTUAL_ENV").map(String::as_str),
        Some("/var/lib/capsem/venv")
    );
    assert_eq!(
        env.get("UV_CACHE_DIR").map(String::as_str),
        Some("/var/cache/capsem/uv"),
        "uv cache must stay off the VirtioFS workspace"
    );
    assert!(
        env.get("PATH")
            .map(|path| {
                path.split(':')
                    .any(|entry| entry == "/var/lib/capsem/venv/bin")
            })
            .unwrap_or(false),
        "PATH must include /var/lib/capsem/venv/bin for non-interactive Python workflows"
    );
    let path_entries = env
        .get("PATH")
        .map(|path| path.split(':').collect::<Vec<_>>())
        .unwrap_or_default();
    assert_eq!(
        path_entries.first().copied(),
        Some("/var/lib/capsem/venv/bin"),
        "PATH must prefer the Python venv"
    );
    let root_local = path_entries
        .iter()
        .position(|entry| *entry == "/root/.local/bin")
        .expect("PATH must include /root/.local/bin");
    let opt_ai = path_entries
        .iter()
        .position(|entry| *entry == "/opt/ai-clis/bin")
        .expect("PATH must include /opt/ai-clis/bin");
    assert!(
        root_local < opt_ai,
        "PATH must prefer /root/.local/bin so Capsem wrappers win in non-interactive exec"
    );
    assert_eq!(
        env.get("GEMINI_API_KEY").map(String::as_str),
        Some("gemini-test-key")
    );
    assert!(
        !env.contains_key("GOOGLE_API_KEY"),
        "Gemini CLI warns when GOOGLE_API_KEY is injected alongside GEMINI_API_KEY"
    );

    let files = runtime
        .guest_config
        .files
        .as_ref()
        .expect("Profile V2 guest boot files should be built without legacy settings");
    let paths = files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(paths.contains("/root/.gemini/settings.json"));
    assert!(paths.contains("/root/.gemini/installation_id"));
    assert!(paths.contains("/root/.local/bin/gemini"));
    assert!(paths.contains("/root/.codex/config.toml"));
    assert!(paths.contains("/root/.claude.json"));

    let gemini_wrapper = files
        .iter()
        .find(|file| file.path == "/root/.local/bin/gemini")
        .expect("gemini wrapper should be present");
    assert_eq!(gemini_wrapper.mode, 0o755);
    assert!(gemini_wrapper.content.contains("gemini --yolo"));

    let gemini_settings = files
        .iter()
        .find(|file| file.path == "/root/.gemini/settings.json")
        .expect("gemini settings should be present");
    let gemini_json: serde_json::Value = serde_json::from_str(&gemini_settings.content).unwrap();
    assert_eq!(
        gemini_json["mcpServers"]["local"]["command"].as_str(),
        Some("/run/capsem-mcp-server")
    );

    let claude_state = files
        .iter()
        .find(|file| file.path == "/root/.claude.json")
        .expect("claude state should be present");
    let claude_json: serde_json::Value = serde_json::from_str(&claude_state.content).unwrap();
    assert_eq!(
        claude_json["mcpServers"]["local"]["command"].as_str(),
        Some("/run/capsem-mcp-server")
    );
}

#[test]
fn process_runtime_source_has_no_v1_policy_bridge() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(manifest_dir.join("src/mcp_runtime.rs")).unwrap();
    for forbidden in [
        "MergedPolicies::from_disk",
        "user_config_path",
        "legacy_policies_from_disk_if_user_file_exists",
        "load_runtime_policy_state_with_legacy",
    ] {
        assert!(
            !source.contains(forbidden),
            "capsem-process runtime must not contain V1 policy bridge token {forbidden:?}"
        );
    }

    let vsock_source = std::fs::read_to_string(manifest_dir.join("src/vsock.rs")).unwrap();
    let core_boot_source = std::fs::read_to_string(
        manifest_dir
            .parent()
            .unwrap()
            .join("capsem-core/src/vm/boot.rs"),
    )
    .unwrap();
    for (path, source) in [
        ("crates/capsem-process/src/mcp_runtime.rs", source.as_str()),
        ("crates/capsem-process/src/vsock.rs", vsock_source.as_str()),
        (
            "crates/capsem-core/src/vm/boot.rs",
            core_boot_source.as_str(),
        ),
    ] {
        assert!(
            !source.contains("net::policy_config::GuestConfig")
                && !source.contains("net::policy_config::{\n    GuestConfig")
                && !source.contains("GuestConfig, GuestFile, PolicyCallback"),
            "{path} must import guest boot config from capsem_core::vm::guest_config, not net::policy_config"
        );
    }
}

#[test]
fn load_runtime_policy_state_falls_back_when_vm_effective_attachment_missing() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = load_runtime_policy_state_from_effective(dir.path());

    assert_eq!(runtime.domain_policy.default_action(), Action::Deny);
    assert!(
        !runtime
            .domain_policy
            .allowed_patterns()
            .contains(&"legacy-only.test".to_string()),
        "missing VM-effective settings fallback must not resurrect legacy allowlists"
    );
    assert_eq!(runtime.mcp_policy.default_tool_decision, ToolDecision::Warn);
}
