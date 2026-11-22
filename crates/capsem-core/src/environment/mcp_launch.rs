//! Remote MCP configuration selected from the acknowledged boot revision.
use capsem_config::{
    environment::{EnvironmentMcpDefinition, EnvironmentMcpPermissions, EnvironmentMcps},
    mcp::{McpConfig, McpManualServer},
};
use capsem_proto::{
    mcp_contracts::{BUILTIN_SERVER_NAME, NS_SEP},
    mcp_worker::{MCP_ADMISSION_HEADER, MCP_SERVER_HEADER},
};
use std::collections::{BTreeMap, BTreeSet};

/// Validate that every configured server has a supported, scoped launch path.
pub fn validate_environment_mcp_launch(mcps: &EnvironmentMcps) -> Result<(), String> {
    definitions(mcps)?.try_for_each(|(name, definition)| validate_remote(name, definition))
}

pub(super) fn definitions(
    mcps: &EnvironmentMcps,
) -> Result<impl Iterator<Item = (&String, &EnvironmentMcpDefinition)>, String> {
    let selected: BTreeSet<_> = mcps.catalog.selected.iter().map(|pin| &pin.id).collect();
    let configuration = mcps.catalog_configuration.iter().flat_map(|entries| entries.iter());
    if selected.len() != mcps.catalog.selected.len()
        || !selected.iter().copied().eq(configuration.clone().map(|(name, _)| name))
        || configuration.clone().any(|(id, _)| mcps.servers.contains_key(id))
    {
        return Err("environment MCP catalog requires an exact configuration for every selected server".into());
    }
    Ok(mcps.servers.iter().chain(configuration))
}

pub(super) fn validate_remote(name: &str, definition: &EnvironmentMcpDefinition) -> Result<(), String> {
    if capsem_config::validate_identifier_shape(name).is_err()
        || matches!(name, BUILTIN_SERVER_NAME | "builtin")
        || name.contains(NS_SEP)
    {
        return Err("environment MCP server has an invalid routing identity".into());
    }
    let headers = match definition {
        EnvironmentMcpDefinition::Remote {
            headers,
            binding_id: None,
            ..
        } => headers,
        EnvironmentMcpDefinition::Remote { .. } => {
            return Err("environment MCP credential binding requires scoped admission".into());
        }
        EnvironmentMcpDefinition::Sandbox { .. } => {
            return Err("sandbox MCP server requires a scoped guest supervisor".into());
        }
    };
    definition.validate_for_environment(false)?;
    let mut names = BTreeSet::new();
    for (name, value) in headers {
        let name =
            http::header::HeaderName::from_bytes(name.as_bytes()).map_err(|_| "invalid environment MCP header")?;
        if matches!(
            name.as_str(),
            MCP_ADMISSION_HEADER
                | MCP_SERVER_HEADER
                | "accept"
                | "content-type"
                | "mcp-session-id"
                | "mcp-protocol-version"
                | "last-event-id"
        ) || !names.insert(name.as_str().to_owned())
            || http::HeaderValue::from_str(value).is_err()
        {
            return Err("environment MCP headers must be unique ordinary HTTP headers".into());
        }
    }
    Ok(())
}

pub(super) fn compose(
    definitions: &BTreeMap<String, EnvironmentMcpDefinition>,
    permissions: &BTreeMap<String, EnvironmentMcpPermissions>,
    corporate: Option<&McpConfig>,
) -> Result<McpConfig, String> {
    let mut config = corporate.cloned().unwrap_or_default();
    for (name, definition) in definitions {
        if config.servers.iter().any(|server| &server.name == name) {
            return Err("environment MCP server identity is owned by corporate configuration".into());
        }
        let EnvironmentMcpDefinition::Remote { url, headers, .. } = definition else {
            return Err("invalid environment MCP remote definition".into());
        };
        config.servers.push(McpManualServer {
            name: name.clone(),
            url: url.clone(),
            headers: headers
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
            auth: None,
            enabled: true,
        });
    }
    for (name, permission) in permissions {
        if !permission.enabled && (name == BUILTIN_SERVER_NAME || definitions.contains_key(name)) {
            config.server_enabled.insert(name.clone(), false);
        }
    }
    Ok(config)
}
