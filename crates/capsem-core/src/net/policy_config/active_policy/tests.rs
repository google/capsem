use super::*;
use crate::net::policy_config::{SecurityPluginMode, SecurityRuleAction};

fn parse(input: &str) -> SettingsFile {
    toml::from_str(input).expect("settings TOML parses")
}

fn rule_ids(active: &ActivePolicyFile) -> Vec<String> {
    active
        .compile_security_rule_set()
        .expect("active policy compiles")
        .rules()
        .iter()
        .map(|rule| rule.rule_id.clone())
        .collect()
}

#[test]
fn built_in_defaults_alone_make_a_complete_policy() {
    let active = ActivePolicyFile::from_settings_and_corp(&SettingsFile::default(), &SettingsFile::default())
        .expect("defaults materialize");

    let ids = rule_ids(&active);
    for id in [
        "profiles.rules.default_http",
        "profiles.rules.default_mcp",
        "profiles.rules.default_expose",
        "profiles.rules.default_http_preview",
        "profiles.rules.default_private",
    ] {
        assert!(ids.iter().any(|rule| rule == id), "{id} missing from {ids:?}");
    }
    assert_eq!(active.plugins["credential_broker"].mode, SecurityPluginMode::Rewrite);
    assert_eq!(active.plugins["log_sanitizer"].mode, SecurityPluginMode::Rewrite);
    assert_eq!(active.mcp, None);
}

#[test]
fn settings_rules_apply_and_corp_rules_win_by_id() {
    let settings = parse(
        r#"
[profiles.rules.block_example]
name = "block_example"
action = "block"
match = 'http.host == "example.invalid"'

[profiles.rules.shared]
name = "shared"
action = "block"
match = 'http.host == "shared.invalid"'
"#,
    );
    let corp = parse(
        r#"
[profiles.rules.shared]
name = "shared"
action = "allow"
match = 'http.host == "shared.invalid"'
"#,
    );
    let active = ActivePolicyFile::from_settings_and_corp(&settings, &corp).expect("policy materializes");
    let rules = active.compile_security_rule_set().expect("policy compiles");

    let by_id = |id: &str| {
        rules
            .rules()
            .iter()
            .find(|rule| rule.rule_id == id)
            .unwrap_or_else(|| panic!("{id} missing"))
            .clone()
    };
    assert_eq!(by_id("profiles.rules.block_example").action, SecurityRuleAction::Block);
    assert_eq!(by_id("profiles.rules.shared").action, SecurityRuleAction::Allow);
}

#[test]
fn plugin_modes_layer_builtin_then_settings_then_corp() {
    let settings = parse(
        r#"
[plugins.credential_broker]
mode = "disable"

[plugins.log_sanitizer]
mode = "disable"
"#,
    );
    let corp = parse(
        r#"
[plugins.log_sanitizer]
mode = "rewrite"
"#,
    );
    let active = ActivePolicyFile::from_settings_and_corp(&settings, &corp).expect("policy materializes");

    assert_eq!(active.plugins["credential_broker"].mode, SecurityPluginMode::Disable);
    assert_eq!(active.plugins["log_sanitizer"].mode, SecurityPluginMode::Rewrite);
}

#[test]
fn corp_network_mechanics_reach_the_session() {
    let corp = parse(
        r#"
refresh_policy = "24h"

[settings."vm.resources.log_bodies"]
value = true
modified = "2026-06-14T00:00:00Z"

[settings."vm.resources.max_body_capture"]
value = 8192
modified = "2026-06-14T00:00:00Z"

[settings."security.web.http_upstream_ports"]
value = [80, 3713, 8080]
modified = "2026-06-14T00:00:00Z"

[network.dns]
upstreams = ["127.0.0.1:5353"]
"#,
    );
    let active =
        ActivePolicyFile::from_settings_and_corp(&SettingsFile::default(), &corp).expect("policy materializes");

    assert_eq!(active.network.log_bodies, Some(true));
    assert_eq!(active.network.max_body_capture, Some(8192));
    assert_eq!(active.network.http_upstream_ports, vec![80, 3713, 8080]);
    assert_eq!(active.network.dns.upstreams, vec!["127.0.0.1:5353".to_string()]);
}

#[test]
fn one_runtime_snapshot_compiles_every_active_policy_concern() {
    let active: ActivePolicyFile = toml::from_str(
        r#"
[user_rules.profiles.rules.runtime_http]
name = "runtime_http"
action = "allow"
priority = 10
match = 'http.host == "policy.example"'

[corp_rules]

[plugins.credential_broker]
mode = "rewrite"

[network]
log_bodies = true
http_upstream_ports = [80, 3713]

[network.dns]
upstreams = ["127.0.0.1:5353", "[::1]:5354"]

[network.upstream_overrides."policy.example:443"]
dial = "127.0.0.1:3713"
protocol = "http"

[mcp.server_enabled]
local = false
"#,
    )
    .unwrap();

    let expected_endpoints = active.model_endpoint_registry().unwrap();
    let runtime = active.compile_runtime().unwrap();

    assert!(runtime
        .security_rules
        .rules()
        .iter()
        .any(|rule| rule.rule_id == "profiles.rules.runtime_http"));
    assert_eq!(runtime.plugins["credential_broker"].mode, SecurityPluginMode::Rewrite);
    assert!(!runtime.mcp.server_enabled["local"]);
    assert_eq!(runtime.model_endpoints, expected_endpoints);
    assert!(runtime.network.log_bodies);
    assert_eq!(runtime.network.http_upstream_ports, vec![80, 3713]);
    assert_eq!(
        runtime.network.upstream_overrides["policy.example:443"].dial,
        "127.0.0.1:3713"
    );
    assert_eq!(
        runtime.dns_upstreams,
        vec!["127.0.0.1:5353".parse().unwrap(), "[::1]:5354".parse().unwrap()]
    );
}

#[test]
fn runtime_snapshot_refuses_an_invalid_dns_upstream_without_fallback() {
    let mut active =
        ActivePolicyFile::from_settings_and_corp(&SettingsFile::default(), &SettingsFile::default()).unwrap();
    active.network.dns.upstreams = vec!["resolver.example:domain".into()];

    let error = active.compile_runtime().err().expect("invalid DNS must fail");
    assert!(error.contains("resolver.example:domain"), "{error}");
}

#[test]
fn default_dns_upstreams_are_explicit_in_the_active_policy_snapshot() {
    let active = ActivePolicyFile::from_settings_and_corp(&SettingsFile::default(), &SettingsFile::default()).unwrap();
    assert_eq!(
        active.network.dns.upstreams,
        crate::net::dns::DEFAULT_UPSTREAMS
            .iter()
            .map(|upstream| (*upstream).to_string())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        active.compile_runtime().unwrap().dns_upstreams,
        crate::net::dns::DEFAULT_UPSTREAMS
            .iter()
            .map(|upstream| upstream.parse().unwrap())
            .collect::<Vec<std::net::SocketAddr>>()
    );
}

#[test]
fn mcp_servers_merge_settings_under_corp() {
    let settings = parse(
        r#"
[[mcp.servers]]
name = "wiki"
url = "https://user.example.invalid/mcp"
"#,
    );
    let corp = parse(
        r#"
[mcp.server_enabled]
local = false
"#,
    );
    let active = ActivePolicyFile::from_settings_and_corp(&settings, &corp).expect("policy materializes");
    let mcp = active.mcp.expect("merged MCP config");

    assert_eq!(mcp.servers.len(), 1);
    assert_eq!(mcp.servers[0].name, "wiki");
    assert_eq!(mcp.server_enabled.get("local"), Some(&false));
}

#[test]
fn a_rule_that_does_not_compile_fails_closed() {
    let settings = parse(
        r#"
[profiles.rules.broken]
name = "broken"
action = "block"
match = 'http.host =='
"#,
    );
    assert!(ActivePolicyFile::from_settings_and_corp(&settings, &SettingsFile::default()).is_err());
}

#[test]
fn the_published_file_round_trips_through_toml() {
    let settings = parse(
        r#"
[profiles.rules.block_example]
name = "block_example"
action = "block"
match = 'http.host == "example.invalid"'

[[mcp.servers]]
name = "wiki"
url = "https://user.example.invalid/mcp"
"#,
    );
    let active = ActivePolicyFile::from_settings_and_corp(&settings, &SettingsFile::default()).expect("materializes");
    let serialized = toml::to_string_pretty(&active).expect("serializes");
    let parsed: ActivePolicyFile = toml::from_str(&serialized).expect("parses back");

    assert_eq!(parsed, active);
    assert_eq!(
        active_policy_digest(serialized.as_bytes()),
        active_policy_digest(toml::to_string_pretty(&parsed).unwrap().as_bytes())
    );
}

#[test]
fn an_active_profile_file_is_refused_not_adapted() {
    let legacy = r#"
id = "code"
name = "Code"
description = "Optimized for coding."
revision = "0.6.5"

[profile_rules]
[corp_rules]
[network]
"#;
    let error = toml::from_str::<ActivePolicyFile>(legacy).expect_err("the profile-era shape is refused");
    assert!(error.to_string().contains("unknown field"), "{error}");
}

/// `capsem doctor` and capsem-bench drive a hermetic mock server on the host
/// loopback. The built-in allow must still
/// win over the local network ask, or every doctor request would wait on one.
#[test]
fn the_built_in_policy_lets_the_doctor_reach_its_mock_server() {
    use crate::security_engine::{
        HttpSecurityEvent, IpSecurityEvent, RuntimeSecurityEventType, SecurityEvent, TcpSecurityEvent,
    };
    let active = ActivePolicyFile::from_settings_and_corp(&SettingsFile::default(), &SettingsFile::default())
        .expect("defaults materialize");
    let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest)
        .with_http(HttpSecurityEvent {
            host: Some("127.0.0.1".to_string()),
            path: Some("/echo".to_string()),
            ..Default::default()
        })
        .with_ip(IpSecurityEvent {
            value: Some("127.0.0.1".to_string()),
            version: Some("4".to_string()),
        })
        .with_tcp(TcpSecurityEvent {
            port: Some("3713".to_string()),
        });

    let rules = active.compile_security_rule_set().expect("active policy compiles");
    let evaluation = rules.evaluate(&event).expect("mock server request evaluates");
    let first = evaluation
        .enforcement_rules()
        .first()
        .map(|rule| (rule.rule_id.as_str(), rule.action));

    assert_eq!(
        first,
        Some((
            "profiles.rules.default_000_capsem_mock_server",
            SecurityRuleAction::Allow
        ))
    );
}

#[test]
fn settings_toml_behavior_settings_reach_the_active_policy() {
    // #289: a non-app setting in settings.toml is the user's. It loads, it
    // shapes the session, and a corp value for the same id still wins.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.toml");
    std::fs::write(
        &path,
        r#"
[settings]
"repository.git.identity.author_name" = { value = "Test User", modified = "2026-01-01T00:00:00Z" }
"vm.resources.max_body_capture" = { value = 1024, modified = "2026-01-01T00:00:00Z" }
"vm.resources.log_bodies" = { value = true, modified = "2026-01-01T00:00:00Z" }
"#,
    )
    .unwrap();
    let settings = crate::net::policy_config::load_local_settings_file(&path).expect("settings.toml loads");

    let active =
        ActivePolicyFile::from_settings_and_corp(&settings, &SettingsFile::default()).expect("policy materializes");
    assert_eq!(active.network.max_body_capture, Some(1024));
    assert_eq!(active.network.log_bodies, Some(true));
    let guest = MergedPolicies::from_files(&settings, &SettingsFile::default())
        .expect("policy compiles")
        .guest;
    assert_eq!(
        guest.env.unwrap_or_default().get("GIT_AUTHOR_NAME").map(String::as_str),
        Some("Test User")
    );

    let corp = parse(
        r#"
[settings]
"vm.resources.max_body_capture" = { value = 0, modified = "2026-01-01T00:00:00Z" }
"#,
    );
    let locked = ActivePolicyFile::from_settings_and_corp(&settings, &corp).expect("policy materializes");
    assert_eq!(locked.network.max_body_capture, Some(0), "corp wins");
    assert_eq!(locked.network.log_bodies, Some(true));
}
