//! Published MCP templates describe the values an operator must supply.
use super::{content, CatalogEntry, CatalogEntryMetadata, CatalogEntryStatus, CatalogKind};
use crate::environment::{
    mcps::{ordinary_header_name, remote_endpoint, sandbox_command, validate_definition},
    variable_name, EnvironmentMcpCatalogInput, EnvironmentMcpDefinition,
};
use capsem_proto::{
    CATALOG_MAX_DESCRIPTION_BYTES, ENVIRONMENT_MAX_CONFIGURATION_ENTRIES, ENVIRONMENT_MAX_CONFIGURATION_TEXT_BYTES,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

// The catalog index is a preview. Live discovery uses the MCP transport limits.
const MAX_PUBLISHED_TOOLS: usize = 128;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpCatalogEntry {
    pub metadata: CatalogEntryMetadata,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<McpCatalogDefinition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_tools: Option<Vec<McpPublishedTool>>,
}

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpCatalogKind {
    Remote,
    Sandbox,
}

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum McpCatalogDefinition {
    Remote {
        url: String,
        /// Ordinary header inputs supplied during environment configuration.
        #[serde(default, deserialize_with = "super::collections::mcp_requirements")]
        headers: BTreeMap<String, McpCatalogRequirement>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        secret_binding: Option<McpCatalogRequirement>,
    },
    Sandbox {
        executable: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workdir: Option<String>,
        /// Values supplied as ordinary guest environment variables.
        #[serde(default, deserialize_with = "super::collections::mcp_requirements")]
        variables: BTreeMap<String, McpCatalogRequirement>,
        /// Names of guest variables backed by approved secret bindings.
        #[serde(default, deserialize_with = "super::collections::mcp_requirements")]
        secret_variables: BTreeMap<String, McpCatalogRequirement>,
    },
}

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpCatalogRequirement {
    pub description: String,
    pub required: bool,
}

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpPublishedTool {
    pub name: String,
    #[serde(default)]
    pub description: String,
}

impl CatalogEntry for McpCatalogEntry {
    const KIND: CatalogKind = CatalogKind::Mcp;

    fn metadata(&self) -> &CatalogEntryMetadata {
        &self.metadata
    }

    fn validate_content(&self) -> Result<(), &'static str> {
        if self.metadata.id == "local" {
            return Err("catalog MCP identity is reserved");
        }
        match &self.definition {
            Some(definition) => definition.validate()?,
            None if self.metadata.status == CatalogEntryStatus::Revoked && self.published_tools.is_none() => {
                return Ok(());
            }
            None => return Err("catalog MCP definition is required"),
        }
        if let Some(tools) = &self.published_tools {
            let mut names = BTreeSet::new();
            if tools.len() > MAX_PUBLISHED_TOOLS
                || tools.iter().any(|tool| {
                    !content::label(&tool.name, ENVIRONMENT_MAX_CONFIGURATION_TEXT_BYTES)
                        || !content::text(&tool.description, CATALOG_MAX_DESCRIPTION_BYTES, true)
                        || !names.insert(&tool.name)
                })
            {
                return Err("invalid or duplicate published MCP tool");
            }
        }
        Ok(())
    }
}

impl McpCatalogDefinition {
    pub fn kind(&self) -> McpCatalogKind {
        match self {
            Self::Remote { .. } => McpCatalogKind::Remote,
            Self::Sandbox { .. } => McpCatalogKind::Sandbox,
        }
    }

    /// Compose caller-supplied values with the exact verified template. The
    /// result contains references that still require runtime binding admission.
    pub fn configure(self, input: EnvironmentMcpCatalogInput, image: bool) -> Result<EnvironmentMcpDefinition, String> {
        input.validate()?;
        let definition = match self {
            Self::Remote {
                url,
                headers,
                secret_binding,
            } => {
                let missing_binding = secret_binding.as_ref().is_some_and(|requirement| requirement.required)
                    && input.binding_id.is_none();
                if missing_binding
                    || (secret_binding.is_none() && input.binding_id.is_some())
                    || !input.secrets.is_empty()
                    || !has_declared_inputs(&headers, input.values.keys().map(String::as_str))
                {
                    return Err("remote MCP catalog configuration does not match the declared inputs".into());
                }
                EnvironmentMcpDefinition::Remote {
                    url,
                    headers: input.values,
                    binding_id: input.binding_id,
                }
            }
            Self::Sandbox {
                executable,
                args,
                workdir,
                variables,
                secret_variables,
            } => {
                if input.binding_id.is_some()
                    || !has_declared_inputs(&variables, input.values.keys().map(String::as_str))
                    || !has_declared_inputs(
                        &secret_variables,
                        input.secrets.values().filter_map(|binding| binding.env_name.as_deref()),
                    )
                {
                    return Err("sandbox MCP catalog configuration does not match the declared inputs".into());
                }
                EnvironmentMcpDefinition::Sandbox {
                    executable,
                    args,
                    workdir,
                    variables: input.values,
                    secrets: input.secrets,
                }
            }
        };
        validate_definition(&definition, image)?;
        Ok(definition)
    }

    fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::Remote {
                url,
                headers,
                secret_binding,
            } => {
                remote_endpoint(url).map_err(|_| "invalid catalog MCP endpoint")?;
                let mut names = BTreeSet::new();
                if headers.len() > ENVIRONMENT_MAX_CONFIGURATION_ENTRIES
                    || headers.iter().any(|(name, requirement)| {
                        !ordinary_header_name(name) || !requirement.valid() || !names.insert(name.to_ascii_lowercase())
                    })
                    || secret_binding.as_ref().is_some_and(|requirement| !requirement.valid())
                {
                    return Err("invalid or duplicate remote MCP configuration requirement");
                }
            }
            Self::Sandbox {
                executable,
                args,
                workdir,
                variables,
                secret_variables,
            } => {
                if !sandbox_command(executable, args, workdir.as_deref()) {
                    return Err("invalid catalog sandbox MCP command");
                }
                if variables.len() + secret_variables.len() > ENVIRONMENT_MAX_CONFIGURATION_ENTRIES
                    || variables.keys().any(|name| secret_variables.contains_key(name))
                    || variables
                        .iter()
                        .chain(secret_variables)
                        .any(|(name, requirement)| !variable_name(name) || !requirement.valid())
                {
                    return Err("invalid or duplicate sandbox MCP configuration requirement");
                }
            }
        }
        Ok(())
    }
}

fn has_declared_inputs<'a>(
    requirements: &BTreeMap<String, McpCatalogRequirement>,
    supplied: impl Iterator<Item = &'a str>,
) -> bool {
    let supplied: BTreeSet<_> = supplied.collect();
    supplied.iter().all(|name| requirements.contains_key(*name))
        && requirements
            .iter()
            .all(|(name, requirement)| !requirement.required || supplied.contains(name.as_str()))
}

impl McpCatalogRequirement {
    fn valid(&self) -> bool {
        content::label(&self.description, CATALOG_MAX_DESCRIPTION_BYTES)
    }
}
