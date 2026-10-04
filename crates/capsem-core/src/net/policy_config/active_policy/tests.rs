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
/// loopback. Every profile used to allow it; the built-in allow must still
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
