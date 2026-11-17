use super::*;
use crate::net::policy_config::{ActivePolicyFile, SettingsFile};
use crate::security_engine::{
    McpSecurityEvent, RuntimeSecurityEventType, SecurityActionRegistry, SecurityDecisionKind, SecurityEmitError,
    SecurityEvent, SecurityEventEmitter, SecurityEventEngine,
};
use capsem_config::{SecurityRuleAction, SecurityRuleSet};
use std::sync::Arc;

fn remote() -> EnvironmentMcpDefinition {
    EnvironmentMcpDefinition::Remote {
        url: "https://mcp.example.org/rpc".into(),
        headers: Default::default(),
        binding_id: None,
    }
}
fn event(server: &str, tool: &str) -> SecurityEvent {
    SecurityEvent::new(RuntimeSecurityEventType::McpToolCall).with_mcp(McpSecurityEvent {
        server_name: Some(server.into()),
        tool_call_name: Some(capsem_proto::mcp_contracts::namespace_name(server, tool)),
        ..Default::default()
    })
}
fn actions(rules: &SecurityRuleSet, server: &str, tool: &str) -> Vec<SecurityRuleAction> {
    rules
        .evaluate(&event(server, tool))
        .unwrap()
        .matched_rules()
        .iter()
        .map(|rule| rule.action)
        .collect()
}

#[test]
fn environment_mcp_permissions_use_live_values_without_activating_desired_servers() {
    let mut boot = specification();
    boot.mcps.servers.insert("existing".into(), remote());
    boot.mcps.catalog.selected.push(EnvironmentCatalogPin {
        id: "catalogued".into(),
        version: "1.0.0".into(),
        catalog_reference: "ghcr.io/google/capsem/mcp:stable".into(),
        resolved_digest: format!("sha256:{}", "a".repeat(64)),
    });
    boot.mcps.permissions.insert(
        "existing".into(),
        EnvironmentMcpPermissions {
            default: EnvironmentCapabilityDecision::Allow,
            ..Default::default()
        },
    );
    let mut live = boot.clone();
    live.mcps.catalog.selected[0].id = "deferred-catalog".into();
    live.mcps.permissions.insert(
        "deferred-catalog".into(),
        EnvironmentMcpPermissions {
            default: EnvironmentCapabilityDecision::Allow,
            ..Default::default()
        },
    );
    live.mcps.servers.insert("next-start".into(), remote());
    live.mcps.permissions.insert(
        "next-start".into(),
        EnvironmentMcpPermissions {
            default: EnvironmentCapabilityDecision::Allow,
            ..Default::default()
        },
    );
    live.mcps.permissions.insert(
        "existing".into(),
        EnvironmentMcpPermissions {
            default: EnvironmentCapabilityDecision::Ask,
            tools: [("read".into(), EnvironmentCapabilityDecision::Allow)].into(),
            ..Default::default()
        },
    );
    boot.validate().unwrap();
    live.validate().unwrap();
    let corp = SettingsFile::default();
    let rules = SecurityRuleSet::new(
        EnvironmentPolicyInputs::new(&boot, &live, &corp)
            .compile_mcp_permissions()
            .unwrap(),
    );
    assert_eq!(actions(&rules, "existing", "read"), [SecurityRuleAction::Allow]);
    assert_eq!(actions(&rules, "existing", "write"), [SecurityRuleAction::Ask]);
    assert_eq!(actions(&rules, "local", "echo"), [SecurityRuleAction::Block]);
    assert!(actions(&rules, "next-start", "read").is_empty());
    assert!(actions(&rules, "deferred-catalog", "read").is_empty());
    assert_eq!(actions(&rules, "catalogued", "read"), [SecurityRuleAction::Block]);
    let response = SecurityEvent::new(RuntimeSecurityEventType::McpToolCall).with_mcp(
        McpSecurityEvent {
            server_name: Some("existing".into()),
            tool_call_name: Some("existing__write".into()),
            ..Default::default()
        }
        .with_response_preview(Some("upstream failed")),
    );
    assert!(rules.evaluate(&response).unwrap().matched_rules().is_empty());
    live.mcps.permissions.remove("existing");
    let rules = SecurityRuleSet::new(
        EnvironmentPolicyInputs::new(&boot, &live, &corp)
            .compile_mcp_permissions()
            .unwrap(),
    );
    assert_eq!(actions(&rules, "existing", "read"), [SecurityRuleAction::Block]);
}

#[test]
fn environment_mcp_permissions_reject_ambiguous_or_oversize_boot_scope() {
    let mut boot = specification();
    let corp = SettingsFile::default();
    boot.mcps.servers.insert("local".into(), remote());
    assert!(EnvironmentPolicyInputs::new(&boot, &boot, &corp)
        .compile_mcp_permissions()
        .is_err());
    boot.mcps.servers.clear();
    boot.mcps.catalog.selected.push(EnvironmentCatalogPin {
        id: "duplicate".into(),
        version: "1.0.0".into(),
        catalog_reference: "ghcr.io/google/capsem/mcp:stable".into(),
        resolved_digest: format!("sha256:{}", "a".repeat(64)),
    });
    boot.mcps.servers.insert("duplicate".into(), remote());
    assert!(EnvironmentPolicyInputs::new(&boot, &boot, &corp)
        .compile_mcp_permissions()
        .is_err());
    boot.mcps.catalog.selected.clear();
    boot.mcps.servers.clear();
    for n in 0..=capsem_proto::ENVIRONMENT_MAX_MCP_SERVERS {
        boot.mcps.servers.insert(format!("server-{n}"), remote());
    }
    assert!(EnvironmentPolicyInputs::new(&boot, &boot, &corp)
        .compile_mcp_permissions()
        .is_err());
}

struct IgnoreEvents;
impl SecurityEventEmitter for IgnoreEvents {
    fn emit(&self, _event: SecurityEvent) -> Result<(), SecurityEmitError> {
        Ok(())
    }
}

#[test]
fn environment_mcp_permissions_keep_current_corporate_precedence_in_the_real_engine() {
    let mut boot = specification();
    boot.mcps.servers.insert("existing".into(), remote());
    boot.mcps.permissions.insert(
        "existing".into(),
        EnvironmentMcpPermissions {
            default: EnvironmentCapabilityDecision::Allow,
            ..Default::default()
        },
    );
    let corp:SettingsFile=toml::from_str("[corp.rules.current]\nname='current_corporate_restriction'\naction='block'\nmatch='mcp.server.name == \"existing\"'\n").unwrap();
    for (current, expected) in [
        (SettingsFile::default(), SecurityDecisionKind::Allow),
        (corp, SecurityDecisionKind::Block),
    ] {
        let mut rules = ActivePolicyFile::from_settings_and_corp(&SettingsFile::default(), &current)
            .unwrap()
            .compile_security_rule_set()
            .unwrap()
            .rules()
            .to_vec();
        rules.extend(
            EnvironmentPolicyInputs::new(&boot, &boot, &current)
                .compile_mcp_permissions()
                .unwrap(),
        );
        let engine = SecurityEventEngine::new(SecurityActionRegistry::new(), Arc::new(IgnoreEvents));
        let result = engine
            .apply_matching_rules_and_emit(&SecurityRuleSet::new(rules), event("existing", "read"))
            .unwrap();
        assert_eq!(result.decision.effective, expected);
    }
}
