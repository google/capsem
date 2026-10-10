use super::*;
use capsem_core::net::policy_config::SecurityPluginMode;

#[test]
fn runtime_policy_source_loads_rules_plugins_mcp() {
    let dir = tempfile::tempdir().unwrap();
    let active_path = dir.path().join("vm/active_policy.toml");
    std::fs::create_dir_all(active_path.parent().unwrap()).unwrap();
    std::fs::write(
        &active_path,
        r#"
[corp_rules]
[network]

[user_rules.profiles.rules.runtime_http]
name = "runtime_http"
action = "allow"
priority = 10
match = 'http.host == "policy.example"'

[plugins.credential_broker]
mode = "rewrite"
detection_level = "informational"

[mcp.server_enabled]
local = false
"#,
    )
    .unwrap();

    let runtime = RuntimePolicySource::new(&active_path).load().unwrap();

    assert_eq!(runtime.active_policy_path, active_path);
    assert!(runtime
        .security_rules
        .rules()
        .iter()
        .any(|rule| rule.rule_id == "profiles.rules.runtime_http"));
    assert_eq!(runtime.plugins["credential_broker"].mode, SecurityPluginMode::Rewrite);
    assert!(!runtime.mcp.server_enabled["local"]);
    assert_eq!(runtime.network.http_upstream_ports, vec![80, 3128, 3713, 8080, 11434]);
    let snapshot = runtime.proxy_policy_snapshot();
    assert_eq!(snapshot.digest(), runtime.active_policy_digest);
    assert_eq!(
        snapshot.network().http_upstream_ports,
        runtime.network.http_upstream_ports
    );
    assert!(snapshot
        .security_rules()
        .rules()
        .iter()
        .any(|rule| rule.rule_id == "profiles.rules.runtime_http"));
    assert_eq!(
        snapshot.plugins()["credential_broker"].mode,
        SecurityPluginMode::Rewrite
    );
    assert!(snapshot
        .model_endpoints()
        .provider_for_host("api.anthropic.com")
        .is_some());
}

#[test]
fn runtime_policy_source_loads_corp_rules_and_dns() {
    let dir = tempfile::tempdir().unwrap();
    let active_path = dir.path().join("vm/active_policy.toml");
    std::fs::create_dir_all(active_path.parent().unwrap()).unwrap();
    std::fs::write(
        &active_path,
        r#"
[user_rules.default.http]
name = "default_http"
action = "allow"
priority = "default"
match = 'has(http.host)'

[corp_rules.corp.rules.block_local_deny_target]
name = "block_local_deny_target"
action = "block"
priority = -100
detection_level = "high"
match = 'http.host == "127.0.0.1" && http.path == "/deny-target"'

[network]
log_bodies = true
max_body_capture = 8192
http_upstream_ports = [80, 3713]

[network.dns]
upstreams = ["127.0.0.1:5353"]

[network.router.expose]
connections = 12
setups = 3
rate_per_second = 7
burst = 2
"#,
    )
    .unwrap();

    let runtime = RuntimePolicySource::new(&active_path).load().unwrap();
    let event = serde_json::json!({
        "http": {
            "host": "127.0.0.1",
            "path": "/deny-target"
        }
    });
    let evaluation = runtime.security_rules.evaluate(&event).unwrap();
    let first = evaluation
        .enforcement_rules()
        .into_iter()
        .next()
        .expect("corp rule should match");

    assert_eq!(first.rule_id, "corp.rules.block_local_deny_target");
    assert_eq!(runtime.dns_upstreams, vec!["127.0.0.1:5353".parse().unwrap()]);
    assert!(runtime.network.log_bodies);
    assert_eq!(runtime.network.max_body_capture, 8192);
    assert_eq!(runtime.network.http_upstream_ports, vec![80, 3713]);
    let budgets = &runtime.network.router;
    assert_eq!(budgets.expose.connections, 12);
    assert_eq!(budgets.expose.setups, 3);
    assert_eq!(budgets.expose.rate_per_second, 7);
    assert_eq!(budgets.expose.burst, 2);
    let materialized = capsem_core::net::policy_config::network_config_from_policy_and_dns(
        &runtime.network,
        capsem_core::net::policy_config::DnsNetworkConfig::default(),
    );
    let encoded = toml::to_string(&materialized).unwrap();
    let decoded: capsem_core::net::policy_config::NetworkConfig = toml::from_str(&encoded).unwrap();
    assert_eq!(decoded.router.as_ref(), Some(budgets));
    assert_eq!(first.action, capsem_core::net::policy_config::SecurityRuleAction::Block);
}

#[test]
fn runtime_policy_source_loads_exact_upstream_overrides() {
    let dir = tempfile::tempdir().unwrap();
    let active_path = dir.path().join("vm/active_policy.toml");
    std::fs::create_dir_all(active_path.parent().unwrap()).unwrap();
    std::fs::write(
        &active_path,
        r#"
[user_rules]
[corp_rules]

[network.upstream_overrides."daily-cloudcode-pa.googleapis.com:443"]
dial = "127.0.0.1:3713"
protocol = "http"
"#,
    )
    .unwrap();

    let runtime = RuntimePolicySource::new(&active_path).load().unwrap();
    let override_route = runtime
        .network
        .find_upstream_override("daily-cloudcode-pa.googleapis.com", 443)
        .expect("exact override should load");

    assert_eq!(override_route.dial, "127.0.0.1:3713");
    assert_eq!(
        override_route.protocol,
        capsem_core::net::policy::UpstreamOverrideProtocol::Http
    );
    assert!(runtime
        .network
        .find_upstream_override("daily-cloudcode-pa.googleapis.com", 80)
        .is_none());
    assert!(runtime.network.find_upstream_override("evil.example", 443).is_none());
}

#[test]
fn invalid_runtime_snapshot_names_its_active_policy_without_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let active_path = dir.path().join("vm/active_policy.toml");
    std::fs::create_dir_all(active_path.parent().unwrap()).unwrap();
    std::fs::write(
        &active_path,
        r#"
[user_rules]
[corp_rules]
[network]
[network.dns]
upstreams = ["resolver.example:domain"]
"#,
    )
    .unwrap();

    let error = RuntimePolicySource::new(&active_path).load().unwrap_err();
    let chain = format!("{error:#}");
    assert!(chain.contains(&active_path.display().to_string()), "{chain}");
    assert!(chain.contains("resolver.example:domain"), "{chain}");
}
