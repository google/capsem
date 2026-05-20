use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::sync::{Mutex, OnceLock};

use capsem_core::mcp::policy::ToolDecision;
use capsem_core::net::domain_policy::{Action, DomainPolicy};
use capsem_core::net::policy::PolicyCallback;
use capsem_core::settings_profiles::{
    CapabilityMode, EffectiveRule, McpConnectorCapsemMetadata, McpConnectorConfig, RuleDecision,
};

use capsem_core::mcp::policy::McpUserConfig;

use super::{
    build_builtin_env, build_servers_with_builtin, insert_builtin_domain_policy_env,
    load_runtime_policy_state, load_runtime_policy_state_from_effective,
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
        condition: "request.host == \"example.com\"".to_string(),
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
        condition: "request.host == \"example.com\" && request.path == \"/secret\"".to_string(),
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
        condition: "request.host == \"bad.example\"".to_string(),
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
        vec!["bad.example".to_string()]
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
        !runtime
            .domain_policy
            .blocked_patterns()
            .contains(&"example.com".to_string()),
        "path-scoped HTTP blocks must not become full-domain DNS blocks"
    );

    let mcp_rules = runtime
        .policy
        .rules_for_callback(PolicyCallback::McpRequest);
    assert_eq!(mcp_rules.len(), 2);
    assert!(mcp_rules
        .iter()
        .any(|(name, _)| *name == "block-prod-delete"));
    assert_eq!(
        mcp_rules
            .iter()
            .find(|(name, _)| *name == "block-prod-delete")
            .unwrap()
            .1
            .decision,
        capsem_core::net::policy::PolicyDecisionKind::Block
    );

    let http_rules = runtime
        .policy
        .rules_for_callback(PolicyCallback::HttpResponse);
    assert_eq!(http_rules.len(), 1);
    assert!(http_rules[0]
        .1
        .condition
        .contains("response.text.contains(\"secret\")"));

    let http_read_rules = runtime.policy.rules_for_callback(PolicyCallback::HttpRead);
    assert!(http_read_rules
        .iter()
        .any(|(name, rule)| *name == "user-read" && rule.condition == "true"));
    let http_write_rules = runtime.policy.rules_for_callback(PolicyCallback::HttpWrite);
    assert!(http_write_rules
        .iter()
        .any(|(name, rule)| *name == "user-write" && rule.condition == "true"));
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
    let effective = capsem_core::settings_profiles::resolve_effective_vm_settings(&roots, None)
        .expect("default effective profile should resolve");
    capsem_core::settings_profiles::write_vm_effective_settings(&session_dir, &effective).unwrap();

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
        Some("0")
    );
    assert_eq!(env.get("TERM").map(String::as_str), Some("xterm-256color"));
    assert_eq!(env.get("LANG").map(String::as_str), Some("C"));
    assert!(
        env.get("PATH")
            .map(|path| path.split(':').any(|entry| entry == "/opt/ai-clis/bin"))
            .unwrap_or(false),
        "PATH must include /opt/ai-clis/bin for npm-installed AI CLIs"
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
    assert!(paths.contains("/root/.codex/config.toml"));
    assert!(paths.contains("/root/.claude.json"));

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
    assert!(runtime
        .domain_policy
        .allowed_patterns()
        .contains(&"elie.net".to_string()));
    assert!(runtime
        .domain_policy
        .allowed_patterns()
        .contains(&"*.elie.net".to_string()));
    assert_eq!(runtime.mcp_policy.default_tool_decision, ToolDecision::Warn);
}

#[test]
fn load_runtime_policy_state_drops_legacy_dns_query_callback() {
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("session");
    std::fs::create_dir_all(&session_dir).unwrap();

    let roots = capsem_core::settings_profiles::ProfileRootSettings::default();
    let mut effective =
        capsem_core::settings_profiles::resolve_effective_vm_settings(&roots, None).unwrap();
    let provenance = effective.profile.provenance.clone();

    effective.rules.push(EffectiveRule {
        id: "dns.legacy".to_string(),
        callback: "dns.query".to_string(),
        condition: "qname == \"example.com\"".to_string(),
        decision: RuleDecision::Block,
        priority: 1,
        rewrite_target: None,
        rewrite_value: None,
        strip_request_headers: Vec::new(),
        strip_response_headers: Vec::new(),
        reason: Some("legacy callback must not enter runtime".to_string()),
        derived: false,
        provenance: provenance.clone(),
        owner_setting_path: None,
        owner_setting_label: None,
        editable: true,
    });
    effective.rules.push(EffectiveRule {
        id: "dns.modern".to_string(),
        callback: "dns.request".to_string(),
        condition: "qname == \"example.com\"".to_string(),
        decision: RuleDecision::Block,
        priority: 1,
        rewrite_target: None,
        rewrite_value: None,
        strip_request_headers: Vec::new(),
        strip_response_headers: Vec::new(),
        reason: Some("modern callback survives runtime conversion".to_string()),
        derived: false,
        provenance,
        owner_setting_path: None,
        owner_setting_label: None,
        editable: true,
    });

    capsem_core::settings_profiles::write_vm_effective_settings(&session_dir, &effective).unwrap();

    let runtime = load_runtime_policy_state_from_effective(&session_dir);

    let dns_rules = runtime.policy.rules_for_callback(PolicyCallback::DnsQuery);
    assert!(
        !dns_rules.iter().any(|(name, _)| *name == "legacy"),
        "legacy dns.query must be dropped while dns.request survives"
    );
    assert!(dns_rules.iter().any(|(name, _)| *name == "modern"));
}
