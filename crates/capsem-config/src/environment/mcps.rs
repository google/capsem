//! Approved MCP configuration describes remote endpoints or guest processes.
//! It never carries a host executable, descriptor, or raw credential material.
use super::*;

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, bound(deserialize = "C: Deserialize<'de>, M: Deserialize<'de>"))]
pub struct EnvironmentMcps<C = EnvironmentCatalogPin, M = EnvironmentMcpDefinition> {
    #[serde(default)]
    pub catalog: EnvironmentCatalogSelections<C>,
    /// Configuration for catalog selections keyed by entry ID. Saved revisions
    /// carry the complete definition resolved by the host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_configuration: Option<BTreeMap<String, M>>,
    #[serde(default)]
    pub servers: BTreeMap<String, EnvironmentMcpDefinition>,
    #[serde(default)]
    pub permissions: BTreeMap<String, EnvironmentMcpPermissions>,
}
impl<C, M> Default for EnvironmentMcps<C, M> {
    fn default() -> Self {
        Self {
            catalog: Default::default(),
            catalog_configuration: None,
            servers: BTreeMap::new(),
            permissions: BTreeMap::new(),
        }
    }
}

/// Values an operator can supply for a catalog definition. The host supplies
/// its remote endpoint or sandbox command when resolving the exact pin.
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentMcpCatalogInput {
    /// Ordinary HTTP headers or guest variables declared by the template.
    #[serde(default)]
    pub values: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding_id: Option<String>,
    /// Sandbox secret references with an explicit declared target variable.
    #[serde(default)]
    pub secrets: BTreeMap<String, EnvironmentSecretBinding>,
}

impl EnvironmentMcpCatalogInput {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.values.len() > capsem_proto::ENVIRONMENT_MAX_CONFIGURATION_ENTRIES
            || self.values.iter().any(|(name, value)| {
                !(ordinary_header_name(name) || validation::variable_name(name))
                    || value.len() > capsem_proto::ENVIRONMENT_MAX_VARIABLE_VALUE_BYTES
                    || value.contains('\0')
            })
            || self
                .binding_id
                .as_deref()
                .is_some_and(|id| !validation::canonical_uuid(id))
            || self.secrets.values().any(|binding| binding.env_name.is_none())
        {
            return Err("invalid MCP catalog configuration inputs".into());
        }
        validation::validate_values(&BTreeMap::new(), &self.secrets)
    }

    pub(super) fn is_empty(&self) -> bool {
        self.values.is_empty() && self.binding_id.is_none() && self.secrets.is_empty()
    }
}

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EnvironmentMcpDefinition {
    Remote {
        url: String,
        #[serde(default)]
        headers: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        binding_id: Option<String>,
    },
    Sandbox {
        executable: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workdir: Option<String>,
        #[serde(default)]
        variables: BTreeMap<String, String>,
        #[serde(default)]
        secrets: BTreeMap<String, EnvironmentSecretBinding>,
    },
}

impl EnvironmentMcpDefinition {
    /// Validate the structural configuration independently of launch admission.
    pub fn validate_for_environment(&self, has_image: bool) -> Result<(), String> {
        validate_definition(self, has_image)
    }

    pub(super) fn has_same_catalog_template(&self, saved: &Self) -> bool {
        match (self, saved) {
            (Self::Remote { url, .. }, Self::Remote { url: previous, .. }) => url == previous,
            (
                Self::Sandbox {
                    executable,
                    args,
                    workdir,
                    ..
                },
                Self::Sandbox {
                    executable: previous,
                    args: previous_args,
                    workdir: previous_workdir,
                    ..
                },
            ) => (executable, args, workdir) == (previous, previous_args, previous_workdir),
            _ => false,
        }
    }

    /// Recover the editable values and references from a materialized definition.
    pub fn into_catalog_input(self) -> EnvironmentMcpCatalogInput {
        match self {
            Self::Remote {
                headers, binding_id, ..
            } => EnvironmentMcpCatalogInput {
                values: headers,
                binding_id,
                ..Default::default()
            },
            Self::Sandbox { variables, secrets, .. } => EnvironmentMcpCatalogInput {
                values: variables,
                secrets,
                ..Default::default()
            },
        }
    }
}

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentCapabilityDecision {
    Allow,
    Ask,
    #[default]
    Block,
}

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentMcpPermissions {
    #[serde(default = "enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub default: EnvironmentCapabilityDecision,
    #[serde(default)]
    pub tools: BTreeMap<String, EnvironmentCapabilityDecision>,
}
impl Default for EnvironmentMcpPermissions {
    fn default() -> Self {
        Self {
            enabled: true,
            default: Default::default(),
            tools: BTreeMap::new(),
        }
    }
}
impl EnvironmentMcpPermissions {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.tools.len() > capsem_proto::ENVIRONMENT_MAX_MCP_TOOL_OVERRIDES {
            return Err("too many environment MCP tool overrides");
        }
        if self.tools.keys().any(|name| name.is_empty() || !text(name)) {
            return Err("invalid environment MCP tool override name");
        }
        Ok(())
    }
}
fn enabled() -> bool {
    true
}

fn text(value: &str) -> bool {
    value.len() <= capsem_proto::ENVIRONMENT_MAX_CONFIGURATION_TEXT_BYTES && !value.chars().any(char::is_control)
}
pub(crate) fn remote_endpoint(url: &str) -> Result<(), &'static str> {
    if !text(url) || url.chars().any(char::is_whitespace) {
        return Err("invalid environment MCP endpoint");
    }
    let endpoint = url::Url::parse(url).map_err(|_| "invalid environment MCP endpoint")?;
    if !matches!(endpoint.scheme(), "https" | "http")
        || endpoint.host_str().is_none()
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.fragment().is_some()
    {
        return Err("invalid environment MCP endpoint or binding");
    }
    Ok(())
}

pub(crate) fn ordinary_header_name(name: &str) -> bool {
    let token = !name.is_empty()
        && name.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'\x60'
                        | b'|'
                        | b'~'
                )
        });
    let reserved = matches!(
        name.to_ascii_lowercase().as_str(),
        "host"
            | "connection"
            | "transfer-encoding"
            | "content-length"
            | "upgrade"
            | "proxy-connection"
            | "te"
            | "trailer"
    );
    text(name) && token && !reserved && !crate::mcp::is_secret_header(name)
}

pub(crate) fn sandbox_command(executable: &str, args: &[String], workdir: Option<&str>) -> bool {
    !executable.is_empty()
        && text(executable)
        && args.len() <= capsem_proto::ENVIRONMENT_MAX_MCP_ARGUMENTS
        && args
            .iter()
            .all(|arg| arg.len() <= capsem_proto::ENVIRONMENT_MAX_CONFIGURATION_TEXT_BYTES && !arg.contains('\0'))
        && workdir.is_none_or(|path| {
            text(path) && path.starts_with('/') && path.split('/').all(|part| !matches!(part, "." | ".."))
        })
}

pub(crate) fn validate_definition(definition: &EnvironmentMcpDefinition, image: bool) -> Result<(), String> {
    match definition {
        EnvironmentMcpDefinition::Remote {
            url,
            headers,
            binding_id,
        } => {
            remote_endpoint(url)?;
            if headers.len() > capsem_proto::ENVIRONMENT_MAX_CONFIGURATION_ENTRIES
                || binding_id.as_deref().is_some_and(|id| !validation::canonical_uuid(id))
            {
                return Err("invalid environment MCP endpoint or binding".into());
            }
            for (name, value) in headers {
                if !text(name) || !text(value) {
                    return Err("environment MCP headers exceed their configuration bounds".into());
                }
                if !ordinary_header_name(name) {
                    return Err("environment MCP headers must be ordinary transport-safe metadata; use a binding for authentication".into());
                }
            }
        }
        EnvironmentMcpDefinition::Sandbox {
            executable,
            args,
            workdir,
            variables,
            secrets,
        } => {
            if !image || !sandbox_command(executable, args, workdir.as_deref()) {
                return Err("invalid sandbox MCP executable, arguments or workdir".into());
            }
            validation::validate_values(variables, secrets)?;
        }
    }
    Ok(())
}

pub(super) fn validate<'a, C, M>(
    mcps: &EnvironmentMcps<C, M>,
    selected: impl Iterator<Item = &'a str>,
    image: bool,
) -> Result<(), String> {
    let mut ids: std::collections::BTreeSet<_> = selected.collect();
    if ids.contains("local") || ids.len() + mcps.servers.len() > capsem_proto::ENVIRONMENT_MAX_MCP_SERVERS {
        return Err("reserved or too many environment MCP servers".into());
    }
    if mcps
        .catalog_configuration
        .as_ref()
        .is_some_and(|configuration| !configuration.keys().all(|id| ids.contains(id.as_str())))
    {
        return Err("MCP catalog configuration requires a selected entry".into());
    }
    for (id, server) in &mcps.servers {
        if id == "local" || crate::validate_identifier_shape(id).is_err() || !ids.insert(id) {
            return Err("invalid or duplicate environment MCP server identity".into());
        }
        validate_definition(server, image)?;
    }
    // The built-in host server retains its reserved identity and scoped policy.
    // A catalog or inline definition cannot replace its implementation.
    ids.insert("local");
    if mcps.permissions.len() > capsem_proto::ENVIRONMENT_MAX_MCP_SERVERS + 1 {
        return Err("too many environment MCP permission entries".into());
    }
    for (id, permissions) in &mcps.permissions {
        if !ids.contains(id.as_str()) {
            return Err("environment MCP permissions require a configured server within its override limit".into());
        }
        permissions.validate()?;
    }
    Ok(())
}
