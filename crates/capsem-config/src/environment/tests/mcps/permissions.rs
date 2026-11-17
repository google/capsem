use super::*;
use crate::{SecurityRuleAction, SecurityRuleSet};

fn compile_environment_mcp_permissions(
    server: &str,
    permissions: &EnvironmentMcpPermissions,
) -> Result<SecurityRuleSet, String> {
    crate::compile_environment_mcp_permissions(server, permissions).map(SecurityRuleSet::new)
}

fn matched(rules: &SecurityRuleSet, server: &str, tool: &str, response: bool) -> Vec<SecurityRuleAction> {
    let event = serde_json::json!({"mcp":{"server":{"name":server},"tool_call":{"name":capsem_proto::mcp_contracts::namespace_name(server, tool)},"response":{"valid":response}}});
    rules
        .evaluate(&event)
        .unwrap()
        .matched_rules()
        .iter()
        .map(|rule| rule.action)
        .collect()
}

#[test]
fn environment_mcp_permissions_explicit_tools_never_match_the_server_fallback() {
    use EnvironmentCapabilityDecision::{Allow, Ask, Block};
    for default in [Allow, Ask, Block] {
        let permissions = EnvironmentMcpPermissions {
            default,
            tools: [("read".into(), Allow), ("review".into(), Ask), ("write".into(), Block)].into(),
            ..Default::default()
        };
        let rules = compile_environment_mcp_permissions("remote", &permissions).unwrap();
        for (tool, expected) in [
            ("read", SecurityRuleAction::Allow),
            ("review", SecurityRuleAction::Ask),
            ("write", SecurityRuleAction::Block),
            (
                "other",
                match default {
                    Allow => SecurityRuleAction::Allow,
                    Ask => SecurityRuleAction::Ask,
                    Block => SecurityRuleAction::Block,
                },
            ),
        ] {
            assert_eq!(matched(&rules, "remote", tool, false), [expected], "{default:?}/{tool}");
            assert!(
                matched(&rules, "remote", tool, true).is_empty(),
                "invocation permission cannot ask again on a response"
            );
            assert!(matched(&rules, "other-server", tool, false).is_empty());
        }
        assert!(rules.rules().iter().all(|rule| !rule.corp_locked
            && rule.priority == crate::MANAGED_PERMISSION_PRIORITY
            && rule.detection_level.is_none()));
        assert!(
            rules
                .evaluate(&serde_json::json!({"mcp":{"server":{"name":"remote"},"response":{"valid":false}}}))
                .unwrap()
                .matched_rules()
                .is_empty(),
            "discovery without a tool call is not an invocation permission"
        );
    }
}

#[test]
fn environment_mcp_permissions_disabled_servers_keep_no_allowing_tool_override() {
    let permissions = EnvironmentMcpPermissions {
        enabled: false,
        default: EnvironmentCapabilityDecision::Allow,
        tools: [("read".into(), EnvironmentCapabilityDecision::Allow)].into(),
    };
    let rules = compile_environment_mcp_permissions("remote", &permissions).unwrap();
    assert_eq!(rules.rules().len(), 1);
    for tool in ["read", "other"] {
        assert_eq!(matched(&rules, "remote", tool, false), [SecurityRuleAction::Block]);
    }
}

#[test]
fn environment_mcp_permissions_escape_tool_names_and_refuse_oversize_without_partial_rules() {
    let tricky = "read\" || mcp.server.name == \"other";
    let mut permissions = EnvironmentMcpPermissions {
        tools: [(tricky.into(), EnvironmentCapabilityDecision::Allow)].into(),
        ..Default::default()
    };
    let rules = compile_environment_mcp_permissions("remote", &permissions).unwrap();
    assert_eq!(matched(&rules, "remote", tricky, false), [SecurityRuleAction::Allow]);
    assert_eq!(matched(&rules, "remote", "other", false), [SecurityRuleAction::Block]);
    assert!(matched(&rules, "other", "other", false).is_empty());
    assert!(compile_environment_mcp_permissions("remote\" || private-canary", &permissions).is_err());
    permissions
        .tools
        .insert("private-canary\n".into(), EnvironmentCapabilityDecision::Allow);
    assert!(!compile_environment_mcp_permissions("remote", &permissions)
        .unwrap_err()
        .contains("canary"));
    permissions.tools.clear();
    for n in 0..5 {
        permissions.tools.insert(
            format!(
                "{n}{}",
                "x".repeat(capsem_proto::ENVIRONMENT_MAX_CONFIGURATION_TEXT_BYTES - 1)
            ),
            EnvironmentCapabilityDecision::Allow,
        );
    }
    assert!(
        compile_environment_mcp_permissions("remote", &permissions).is_err(),
        "the complete fallback must fail explicitly when its condition exceeds the shared limit"
    );
    permissions.tools.clear();
    for n in 0..capsem_proto::ENVIRONMENT_MAX_MCP_TOOL_OVERRIDES {
        permissions
            .tools
            .insert(format!("tool-{n}"), EnvironmentCapabilityDecision::Allow);
    }
    assert!(compile_environment_mcp_permissions("remote", &permissions).is_ok());
    permissions
        .tools
        .insert("extra".into(), EnvironmentCapabilityDecision::Allow);
    assert!(compile_environment_mcp_permissions("remote", &permissions).is_err());
}
