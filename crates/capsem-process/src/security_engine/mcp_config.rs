use std::collections::HashMap;

use capsem_core::mcp::policy::{McpManualServer, McpUserConfig, ToolDecision};
use capsem_core::settings_profiles::{self, CapabilityMode};

pub(super) fn mcp_user_config_from_effective(
    effective: &settings_profiles::EffectiveVmSettings,
) -> McpUserConfig {
    let default_tool_permission = Some(match effective.security.value.capabilities.mcp_tools {
        CapabilityMode::Allow | CapabilityMode::Audit => ToolDecision::Allow,
        CapabilityMode::Ask => ToolDecision::Warn,
        CapabilityMode::Block => ToolDecision::Block,
    });

    let servers = effective
        .mcp
        .value
        .connectors
        .iter()
        .map(|(id, connector)| McpManualServer {
            name: id.clone(),
            url: connector.url.clone().unwrap_or_default(),
            command: connector.command.clone(),
            args: connector.args.clone(),
            env: connector
                .env
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            headers: connector
                .headers
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            bearer_token: connector.bearer_token.clone(),
            pool_size: connector.pool_size,
            pool_safe_tools: connector.pool_safe_tools.clone(),
            enabled: connector.enabled,
        })
        .collect::<Vec<_>>();

    let server_enabled = effective
        .mcp
        .value
        .connectors
        .iter()
        .map(|(id, connector)| (id.clone(), connector.enabled))
        .collect::<HashMap<_, _>>();

    McpUserConfig {
        global_policy: None,
        default_tool_permission,
        health_check_interval_secs: None,
        servers,
        server_enabled,
        tool_permissions: HashMap::new(),
        audit_rules: Vec::new(),
    }
}
