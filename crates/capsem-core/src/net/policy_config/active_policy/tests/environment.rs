use super::*;
use crate::environment::{EnvironmentPolicyInputs, PluginDirection};
use crate::security_engine::{evaluate_security_boundary, HttpSecurityEvent, RuntimeSecurityEventType, SecurityEvent};
use capsem_config::environment::EnvironmentSpecification;

fn specification() -> EnvironmentSpecification {
    toml::from_str("name='development'\n[image]\nreference='development:stable'\nresolved_digest='sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'").unwrap()
}

#[test]
fn environment_policy_roundtrip_enforces_live_access_boot_detection_and_directional_plugins() {
    let mut boot = specification();
    boot.access.custom.insert(
        "endpoint".into(),
        toml::from_str("name='Endpoint'\naction='allow'\nmatch='http.host == \"scoped.example\"'").unwrap(),
    );
    boot.detection.custom.insert(
        "observe".into(),
        toml::from_str("name='Observe'\nlevel='high'\nmatch='http.host == \"scoped.example\"'").unwrap(),
    );
    boot.plugins = toml::from_str("[pii_filter.inbound]\nmode='block'\n[pii_filter.inbound.options]\nemail=false\n[pii_filter.outbound]\nmode='block'").unwrap();
    let mut live = boot.clone();
    live.access.custom.get_mut("endpoint").unwrap().action = SecurityRuleAction::Block;
    live.detection.custom.get_mut("observe").unwrap().level = capsem_config::DetectionLevel::Critical;
    live.plugins = Default::default();
    let corp = parse("[corp.rules.mandatory]\nname='mandatory'\naction='block'\nmatch='http.host == \"corp.example\"'\n[environment_plugins.utf8_guard]\nmode='block'\n[environment_plugins.utf8_guard.options]\ninbound=false\noutbound=true");
    let active = ActivePolicyFile::from_environment(&EnvironmentPolicyInputs::new(&boot, &live, &corp), None).unwrap();
    let bytes = toml::to_string_pretty(&active).unwrap();
    let exact: ActivePolicyFile = toml::from_str(&bytes).unwrap();
    let runtime = exact.compile_runtime().unwrap();
    let policy = exact.compile_security_rule_set().unwrap();
    assert_eq!(policy.rules().len(), runtime.security_rules.rules().len());
    for (host, inbound, allowed) in [
        ("scoped.example", true, false),
        ("corp.example", true, false),
        ("other.example", true, true),
        ("other.example", false, false),
    ] {
        let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http(HttpSecurityEvent {
            host: Some(host.into()),
            status: inbound.then(|| "200".into()),
            body: Some("person@example.com".into()),
            ..Default::default()
        });
        let result = evaluate_security_boundary(&runtime.security_rules, runtime.plugins.clone(), event).unwrap();
        assert_eq!(result.enforcement.is_allowed(), allowed, "{host}, inbound={inbound}");
        if host == "scoped.example" {
            let detection = result
                .event
                .detections
                .iter()
                .find(|d| d.rule_id.as_deref() == Some("environment.detection.observe"))
                .unwrap();
            assert_eq!(detection.detection_level, capsem_config::DetectionLevel::High);
        }
    }
    let utf8 = runtime
        .plugins
        .configuration("utf8_guard", PluginDirection::Inbound)
        .unwrap();
    assert_eq!(utf8.mode, SecurityPluginMode::Disable);
    assert!(utf8.locked());
    let mut ambiguous = exact.clone();
    ambiguous.plugins.insert("pii_filter".into(), Default::default());
    assert!(ambiguous.compile_runtime().is_err());
    let mut legacy_rules = exact;
    legacy_rules.user_rules = SecurityRuleProfile::parse_toml(
        "[profiles.rules.injected]\nname='injected'\naction='allow'\nmatch='http.host == \"injected.example\"'",
    )
    .unwrap();
    assert!(legacy_rules.compile_runtime().is_err());
}

#[test]
fn environment_policy_refuses_legacy_corporate_plugin_modes() {
    let boot = specification();
    let corporate = parse("[plugins.pii_filter]\nmode='block'");
    assert!(ActivePolicyFile::from_environment(&EnvironmentPolicyInputs::new(&boot, &boot, &corporate), None).is_err());
}

#[test]
fn environment_policy_roundtrip_uses_boot_mcp_scope_and_live_permissions() {
    use crate::security_engine::McpSecurityEvent;
    use capsem_config::environment::*;
    let mut boot = specification();
    boot.mcps.servers.insert(
        "remote".into(),
        toml::from_str("kind='remote'\nurl='https://mcp.example/rpc'").unwrap(),
    );
    let mut live = boot.clone();
    live.mcps.permissions.insert(
        "remote".into(),
        EnvironmentMcpPermissions {
            default: EnvironmentCapabilityDecision::Block,
            tools: [("read".into(), EnvironmentCapabilityDecision::Allow)].into(),
            ..Default::default()
        },
    );
    live.mcps.servers.insert(
        "deferred".into(),
        toml::from_str("kind='remote'\nurl='https://deferred.example/rpc'").unwrap(),
    );
    live.mcps.permissions.insert(
        "deferred".into(),
        EnvironmentMcpPermissions {
            default: EnvironmentCapabilityDecision::Allow,
            ..Default::default()
        },
    );
    let corporate = SettingsFile::default();
    let active =
        ActivePolicyFile::from_environment(&EnvironmentPolicyInputs::new(&boot, &live, &corporate), None).unwrap();
    let exact: ActivePolicyFile = toml::from_str(&toml::to_string(&active).unwrap()).unwrap();
    let runtime = exact.compile_runtime().unwrap();
    assert_eq!(runtime.mcp.servers.len(), 1);
    assert_eq!(runtime.mcp.servers[0].name, "remote");
    assert_eq!(runtime.mcp.servers[0].url, "https://mcp.example/rpc");
    assert!(runtime.mcp.servers[0].enabled);
    assert_eq!(runtime.scoped_mcp_servers, BTreeSet::from(["remote".into()]));
    assert!(!runtime
        .security_rules
        .rules()
        .iter()
        .any(|rule| rule.rule_id.contains("deferred")));
    for (tool, allowed) in [("read", true), ("write", false)] {
        let event = SecurityEvent::new(RuntimeSecurityEventType::McpToolCall).with_mcp(McpSecurityEvent {
            server_name: Some("remote".into()),
            tool_call_name: Some(format!("remote__{tool}")),
            ..Default::default()
        });
        let result = evaluate_security_boundary(&runtime.security_rules, runtime.plugins.clone(), event).unwrap();
        assert_eq!(result.enforcement.is_allowed(), allowed);
    }
}

#[test]
fn environment_mcp_runtime_respects_corporate_ownership_and_disablement() {
    use capsem_config::environment::*;
    let mut boot = specification();
    boot.mcps.servers.insert(
        "remote".into(),
        toml::from_str("kind='remote'\nurl='https://mcp.example/rpc'").unwrap(),
    );
    let mut live = boot.clone();
    live.mcps.permissions.insert(
        "remote".into(),
        EnvironmentMcpPermissions {
            enabled: false,
            ..Default::default()
        },
    );
    let corporate = parse("[mcp.server_enabled]\nremote=true\nlocal=false");
    let compile = |boot, live, corporate| {
        ActivePolicyFile::from_environment(&EnvironmentPolicyInputs::new(boot, live, corporate), None)
    };
    let mcp = compile(&boot, &live, &corporate)
        .unwrap()
        .compile_runtime()
        .unwrap()
        .mcp;
    assert!(!mcp.server_enabled["remote"]);
    assert!(!mcp.server_enabled["local"]);
    let corporate = parse("[mcp.server_enabled]\nremote=false");
    assert!(
        !compile(&boot, &boot, &corporate)
            .unwrap()
            .compile_runtime()
            .unwrap()
            .mcp
            .server_enabled["remote"]
    );
    let collision = parse("[[mcp.servers]]\nname='remote'\nurl='https://different.example/rpc'");
    assert!(compile(&boot, &boot, &collision)
        .unwrap_err()
        .contains("owned by corporate"));
    let mut binding = boot.clone();
    let EnvironmentMcpDefinition::Remote { binding_id, .. } = binding.mcps.servers.get_mut("remote").unwrap() else {
        panic!("fixture must be remote");
    };
    *binding_id = Some("00000000-0000-4000-8000-000000000010".into());
    assert!(compile(&binding, &binding, &corporate)
        .unwrap_err()
        .contains("binding requires scoped admission"));
}
