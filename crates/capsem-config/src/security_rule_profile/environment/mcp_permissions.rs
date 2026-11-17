//! Invocation defaults exclude explicit tool overrides. Observed responses keep
//! their own rules, and definition pinning remains a separate mandatory gate.
use super::*;
use crate::environment::{EnvironmentCapabilityDecision, EnvironmentMcpPermissions};

fn action(decision: EnvironmentCapabilityDecision) -> SecurityRuleAction {
    match decision {
        EnvironmentCapabilityDecision::Allow => SecurityRuleAction::Allow,
        EnvironmentCapabilityDecision::Ask => SecurityRuleAction::Ask,
        EnvironmentCapabilityDecision::Block => SecurityRuleAction::Block,
    }
}

fn rule(
    server: &str,
    key: &str,
    action: SecurityRuleAction,
    condition: String,
) -> Result<CompiledSecurityRule, String> {
    let mut compiled = CompiledSecurityRule::from_environment_access(
        "mcp_permission",
        &EnvironmentAccessRule {
            name: "MCP invocation permission".into(),
            action,
            condition,
            enabled: true,
            priority: Some(MANAGED_PERMISSION_PRIORITY),
        },
    )?;
    compiled.rule_id = format!("environment.mcps.{server}.{key}");
    compiled.namespace = "environment.mcps".into();
    compiled.rule_key = key.into();
    Ok(compiled)
}

/// Compile one configured server's permissions at user priority. Core composes
/// these with current corporate rules. This leaf compiler confers no server
/// availability, definition pin, credential or caller authority.
pub fn compile_environment_mcp_permissions(
    server: &str,
    permissions: &EnvironmentMcpPermissions,
) -> Result<Vec<CompiledSecurityRule>, String> {
    crate::validate_identifier_shape(server).map_err(|_| "invalid environment MCP server identity")?;
    permissions.validate()?;
    let base = format!(
        "mcp.response.valid == \"false\" && mcp.server.name == {}",
        condition_string(server)
    );
    let mut fallback = format!("{base} && has(mcp.tool_call.name)");
    if permissions.enabled {
        for tool in permissions.tools.keys() {
            let name = capsem_proto::mcp_contracts::namespace_name(server, tool);
            let exclusion = format!(" && mcp.tool_call.name != {}", condition_string(&name));
            if fallback.len().saturating_add(exclusion.len()) > capsem_proto::ENVIRONMENT_MAX_RULE_BYTES {
                return Err("environment MCP fallback condition exceeds its byte limit".into());
            }
            fallback.push_str(&exclusion);
        }
    }
    let mut compiled = Vec::with_capacity(if permissions.enabled {
        permissions.tools.len() + 1
    } else {
        1
    });
    compiled.push(rule(
        server,
        "default",
        if permissions.enabled {
            action(permissions.default)
        } else {
            SecurityRuleAction::Block
        },
        fallback,
    )?);
    if permissions.enabled {
        for (index, (tool, decision)) in permissions.tools.iter().enumerate() {
            let name = capsem_proto::mcp_contracts::namespace_name(server, tool);
            let condition = format!("{base} && mcp.tool_call.name == {}", condition_string(&name));
            let mut compiled_rule = rule(server, &format!("tool_{index}"), action(*decision), condition)?;
            compiled_rule.managed = Some(SecurityRuleManagedTarget::McpTool {
                server: server.into(),
                tool: tool.clone(),
                operation: SecurityRuleManagedOperation::Permission,
            });
            compiled.push(compiled_rule);
        }
    }
    Ok(compiled)
}
