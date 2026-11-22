//! MCP launch configuration selected from the acknowledged boot revision.
use capsem_config::{
    environment::{EnvironmentMcpDefinition, EnvironmentMcpPermissions, EnvironmentMcps},
    mcp::{McpConfig, McpManualServer},
};
use capsem_proto::{
    mcp_contracts::{BUILTIN_SERVER_NAME, NS_SEP},
    mcp_worker::{MCP_ADMISSION_HEADER, MCP_SERVER_HEADER},
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// One slot in the acknowledged boot revision, retained by the VM owner.
#[derive(Clone, PartialEq, Eq)]
pub struct SandboxMcp {
    pub name: String,
    pub enabled: bool,
    preface: Arc<[u8]>,
}

#[derive(serde::Serialize)]
struct SandboxMcpSetup<'a> {
    argv: Vec<&'a String>,
    workdir: Option<&'a str>,
    variables: &'a BTreeMap<String, String>,
}

impl SandboxMcp {
    pub fn preface(&self) -> &[u8] {
        &self.preface
    }

    pub fn same_launch(&self, other: &Self) -> bool {
        self.name == other.name && self.preface == other.preface
    }

    pub(super) fn new(name: &str, definition: &EnvironmentMcpDefinition, enabled: bool) -> Result<Self, String> {
        let EnvironmentMcpDefinition::Sandbox {
            executable,
            args,
            workdir,
            variables,
            ..
        } = definition
        else {
            return Err("invalid environment MCP sandbox definition".into());
        };
        // This length-prefixed record is the only data read by the trusted
        // OCI launcher before it gives stdin to the configured program.
        let mut preface = vec![0; 4];
        serde_json::to_writer(
            &mut preface,
            &SandboxMcpSetup {
                argv: std::iter::once(executable).chain(args).collect(),
                workdir: workdir.as_deref(),
                variables,
            },
        )
        .map_err(|_| "could not encode sandbox MCP setup")?;
        let size = preface.len() - 4;
        if size > capsem_proto::ENVIRONMENT_MAX_SPEC_BYTES {
            return Err("environment MCP setup exceeds its byte limit".into());
        }
        preface[..4].copy_from_slice(&(size as u32).to_be_bytes());
        Ok(Self {
            name: name.into(),
            enabled,
            preface: preface.into(),
        })
    }
}

impl std::fmt::Debug for SandboxMcp {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SandboxMcp")
            .field("name", &self.name)
            .field("enabled", &self.enabled)
            .finish_non_exhaustive()
    }
}

/// Validate that every configured server has a supported, scoped launch path.
pub fn validate_environment_mcp_launch(mcps: &EnvironmentMcps, has_workload: bool) -> Result<(), String> {
    definitions(mcps)?.try_for_each(|(name, definition)| validate(name, definition, has_workload))
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

pub(super) fn validate(name: &str, definition: &EnvironmentMcpDefinition, has_workload: bool) -> Result<(), String> {
    if capsem_config::validate_identifier_shape(name).is_err()
        || matches!(name, BUILTIN_SERVER_NAME | "builtin")
        || name.contains(NS_SEP)
    {
        return Err("environment MCP server has an invalid routing identity".into());
    }
    definition.validate_for_environment(has_workload)?;
    let headers = match definition {
        EnvironmentMcpDefinition::Remote {
            headers,
            binding_id: None,
            ..
        } => headers,
        EnvironmentMcpDefinition::Remote { .. } => {
            return Err("environment MCP credential binding requires scoped admission".into());
        }
        EnvironmentMcpDefinition::Sandbox { secrets, .. } if secrets.is_empty() => return Ok(()),
        EnvironmentMcpDefinition::Sandbox { .. } => {
            return Err("sandbox MCP secret bindings require scoped admission".into())
        }
    };
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
    sandbox: &BTreeMap<String, EnvironmentMcpDefinition>,
    permissions: &BTreeMap<String, EnvironmentMcpPermissions>,
    corporate: Option<&McpConfig>,
) -> Result<McpConfig, String> {
    let mut config = corporate.cloned().unwrap_or_default();
    if config
        .servers
        .iter()
        .any(|server| definitions.contains_key(&server.name) || sandbox.contains_key(&server.name))
    {
        return Err("environment MCP server identity is owned by corporate configuration".into());
    }
    for (name, definition) in definitions {
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
        if !permission.enabled
            && (name == BUILTIN_SERVER_NAME || definitions.contains_key(name) || sandbox.contains_key(name))
        {
            config.server_enabled.insert(name.clone(), false);
        }
    }
    Ok(config)
}
