use super::*;
use capsem_core::net::policy_config::SecurityPluginMode;

#[test]
fn native_mcp_definitions_exclude_credentials_host_environment_and_executable_paths() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("active_policy.toml");
    std::fs::write(
        &path,
        format!(
            r#"
        [corp_rules]
        [user_rules]
        [network]
        [[mcp.servers]]
        name = "configured"
        url = "https://configured.example/mcp"
        enabled = true
        headers = {{ "x-private-header" = "host-header-sentinel" }}
        auth = {{ kind = "bearer", credential_ref = "credential:blake3:{}" }}
    "#,
            "a".repeat(64)
        ),
    )
    .unwrap();
    let runtime = RuntimePolicySource::new(&path).load().unwrap();
    let builtin = directory.path().join("capsem-mcp-builtin");
    std::fs::write(&builtin, b"unused definition fixture").unwrap();
    let definitions = runtime.mcp_servers(
        Some(&builtin),
        HashMap::from([
            ("CAPSEM_SESSION_DIR".into(), "/host/session".into()),
            ("UNRELATED_SECRET".into(), "host-env-sentinel".into()),
        ]),
    );
    assert_eq!(definitions.len(), 2);
    for definition in &definitions {
        assert!(definition.headers.is_empty());
        assert!(definition.auth.is_none());
        assert!(definition.env.is_empty());
        assert!(definition.args.is_empty());
    }
    let builtin = definitions
        .iter()
        .find(|definition| definition.name == capsem_proto::mcp_contracts::BUILTIN_SERVER_NAME)
        .unwrap();
    assert_eq!(builtin.command.as_deref(), Some("capsem-mcp-builtin"));
    assert_eq!(builtin.pool_size, Some(1));
    let remote = definitions
        .iter()
        .find(|definition| definition.name == "configured")
        .unwrap();
    assert_eq!(remote.url, "https://configured.example/mcp");
    assert!(remote.command.is_none());
    // A refresh uses the identity admitted before confinement. It cannot
    // stat the host executable again (Seatbelt denies that metadata access).
    std::fs::remove_file(directory.path().join("capsem-mcp-builtin")).unwrap();
    let mut refreshed = runtime;
    refreshed.mcp.server_enabled.insert("local".into(), false);
    let definitions = refreshed.mcp_servers(Some(&directory.path().join("capsem-mcp-builtin")), HashMap::new());
    let builtin = definitions
        .iter()
        .find(|definition| definition.name == "local")
        .unwrap();
    assert!(!builtin.enabled);
    assert!(builtin.env.is_empty());
    assert_eq!(builtin.command.as_deref(), Some("capsem-mcp-builtin"));
}

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
http_upstream_ports = [80, 3128, 3713, 8080, 11434]

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
    assert_eq!(
        runtime
            .plugins
            .configuration("credential_broker", capsem_core::environment::PluginDirection::Outbound)
            .unwrap()
            .mode,
        SecurityPluginMode::Rewrite
    );
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
        snapshot
            .plugins()
            .configuration("credential_broker", capsem_core::environment::PluginDirection::Outbound)
            .unwrap()
            .mode,
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

#[test]
fn environment_policy_reload_retains_the_exact_typed_snapshot_on_failure() {
    let directory = tempfile::tempdir().unwrap();
    let source = RuntimePolicySource::new(directory.path().join("active_policy.toml"));
    let boot = toml::from_str("name='development'\n[plugins.pii_filter.outbound]\nmode='block'\n[plugins.pii_filter.inbound]\nmode='block'\n[plugins.pii_filter.inbound.options]\nemail=false").unwrap();
    let corporate = Default::default();
    let inputs = capsem_core::environment::EnvironmentPolicyInputs::new(&boot, &boot, &corporate);
    let active = ActivePolicyFile::from_environment(&inputs, None).unwrap();
    let bytes = toml::to_string(&active).unwrap().into_bytes();
    let loaded = source.load_bytes(&bytes).unwrap();
    assert_eq!(
        loaded.active_policy_digest,
        capsem_core::net::policy_config::active_policy_digest(&bytes)
    );
    let snapshot = loaded.proxy_policy_snapshot();
    assert!(snapshot.plugins().same_snapshot(&loaded.plugins));
    assert!(source.load_bytes(b"environment = false").is_err());
    let retained = source.current().unwrap();
    assert!(retained.plugins.same_snapshot(&loaded.plugins));
    assert_eq!(retained.active_policy_digest, loaded.active_policy_digest);
    for (inbound, allowed) in [(false, false), (true, true)] {
        let event = capsem_core::security_engine::SecurityEvent::new(
            capsem_core::security_engine::RuntimeSecurityEventType::HttpRequest,
        )
        .with_http(capsem_core::security_engine::HttpSecurityEvent {
            body: Some("person@example.com".into()),
            status: inbound.then(|| "200".into()),
            ..Default::default()
        });
        let result = capsem_core::security_engine::evaluate_security_boundary(
            &retained.security_rules,
            retained.plugins.clone(),
            event,
        )
        .unwrap();
        assert_eq!(result.enforcement.is_allowed(), allowed);
    }
}
