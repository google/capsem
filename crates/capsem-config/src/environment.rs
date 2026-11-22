//! Canonical current environment specifications. The service verifies revision
//! identity against its ledger before loading or publishing these documents.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

mod catalog;
mod drafts;
mod mcps;
mod rules;
pub use mcps::{EnvironmentCapabilityDecision, EnvironmentMcpDefinition, EnvironmentMcpPermissions, EnvironmentMcps};
pub use rules::{
    EnvironmentAccessRule, EnvironmentAccessRules, EnvironmentDetectionRule, EnvironmentDetectionRules,
    EnvironmentRuleParameter, EnvironmentRules,
};
#[cfg(test)]
mod tests;
mod toml_edit;
mod validation;
pub use catalog::{EnvironmentCatalogPin, EnvironmentCatalogSelection, EnvironmentCatalogSelections};

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentIdentity {
    pub schema_version: u16,
    pub id: String,
    pub revision_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_environment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_revision_id: Option<String>,
}

/// Editable content contains no service-assigned revision/history metadata.
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    deny_unknown_fields,
    bound(deserialize = "I: Deserialize<'de>, R: Deserialize<'de>, C: Deserialize<'de>")
)]
pub struct EnvironmentSpecification<I = EnvironmentImage, R = EnvironmentRepository, C = EnvironmentCatalogPin> {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<I>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup: Option<EnvironmentStartup>,
    #[serde(default)]
    pub variables: BTreeMap<String, String>,
    #[serde(default)]
    pub secrets: BTreeMap<String, EnvironmentSecretBinding>,
    #[serde(default)]
    pub repositories: Vec<R>,
    #[serde(default)]
    pub packages: EnvironmentPackages,
    #[serde(default)]
    pub skills: EnvironmentCatalogSelections<C>,
    #[serde(default)]
    #[cfg_attr(feature = "schema", schema(value_type = EnvironmentRules<C, EnvironmentAccessRule>))]
    pub access: EnvironmentAccessRules<C>,
    #[serde(default)]
    #[cfg_attr(feature = "schema", schema(value_type = EnvironmentRules<C, EnvironmentDetectionRule>))]
    pub detection: EnvironmentDetectionRules<C>,
    #[serde(default)]
    pub mcps: EnvironmentMcps<C>,
    #[serde(default)]
    pub plugins: crate::plugins::EnvironmentPlugins,
}

/// Agent/operator intent has selectors and optional content expectations. The host resolves all pins.
pub type EnvironmentDraft =
    EnvironmentSpecification<EnvironmentImageSelection, EnvironmentRepositorySelection, EnvironmentCatalogSelection>;

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentImageSelection {
    pub reference: String,
    #[serde(default)]
    pub update_policy: EnvironmentImageUpdatePolicy,
}

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentRepositorySelection {
    pub url: String,
    #[serde(rename = "ref")]
    pub reference: String,
    pub destination: String,
}

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentImage {
    pub reference: String,
    pub resolved_digest: String,
    #[serde(default)]
    pub update_policy: EnvironmentImageUpdatePolicy,
}

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentImageUpdatePolicy {
    #[default]
    Manual,
    Ask,
    Auto,
}

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentStartup {
    pub script: String,
    pub timeout_seconds: u32,
}

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentSecretBinding {
    pub binding_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_name: Option<String>,
}

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentRepository {
    pub url: String,
    #[serde(rename = "ref")]
    pub reference: String,
    pub resolved_commit: String,
    pub destination: String,
}

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentPackages {
    #[serde(default)]
    pub allows_package_manager: bool,
    #[serde(default)]
    pub registries: Vec<String>,
}

/// Binary history uses this typed envelope; the canonical TOML file is flat.
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentDocument {
    pub identity: EnvironmentIdentity,
    #[cfg_attr(feature = "schema", schema(value_type = EnvironmentSpecification<EnvironmentImage, EnvironmentRepository, EnvironmentCatalogPin>))]
    pub specification: EnvironmentSpecification,
}

impl EnvironmentDocument {
    pub fn from_toml(source: &str) -> Result<Self, String> {
        let (identity, table) = toml_edit::document_tables(source)?;
        let document = Self {
            identity,
            specification: toml::Value::Table(table)
                .try_into()
                .map_err(|_| "invalid environment specification")?,
        };
        // Enforce the canonical serialized byte budget, including escaping.
        document.to_toml()?;
        Ok(document)
    }

    pub fn to_toml(&self) -> Result<String, String> {
        self.validate_fields()?;
        #[derive(Serialize)]
        struct File<'a> {
            #[serde(flatten)]
            identity: &'a EnvironmentIdentity,
            #[serde(flatten)]
            specification: &'a EnvironmentSpecification,
        }
        let encoded = toml::to_string(&File {
            identity: &self.identity,
            specification: &self.specification,
        })
        .map_err(|_| "cannot serialize environment specification")?;
        if encoded.len() > capsem_proto::ENVIRONMENT_MAX_SPEC_BYTES {
            return Err("environment specification exceeds its byte limit".into());
        }
        Ok(encoded)
    }
}

// These representations may contain ordinary variables accidentally holding
// credentials. Debug output reports shape only, never variable/script values.
impl<I, R, C> std::fmt::Debug for EnvironmentSpecification<I, R, C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvironmentSpecification")
            .field("variable_count", &self.variables.len())
            .field("binding_count", &self.secrets.len())
            .field("repository_count", &self.repositories.len())
            .field("skill_count", &self.skills.selected.len())
            .field("mcp_server_count", &self.mcps.servers.len())
            .field("has_image", &self.image.is_some())
            .field("has_startup", &self.startup.is_some())
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for EnvironmentDocument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvironmentDocument")
            .field("specification", &self.specification)
            .finish_non_exhaustive()
    }
}

/// The API can expose these exact compiled bounds to SDKs and editors.
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct EnvironmentLimits {
    pub max_name_bytes: usize,
    pub max_description_bytes: usize,
    pub max_startup_script_bytes: usize,
    pub max_variables: usize,
    pub max_variable_name_bytes: usize,
    pub max_variable_value_bytes: usize,
    pub max_variable_total_bytes: usize,
    pub max_secret_bindings: usize,
    pub max_repositories: usize,
    pub max_spec_bytes: usize,
    pub max_catalog_selections: usize,
    pub max_catalog_reference_bytes: usize,
    pub max_rules: usize,
    pub max_rule_bytes: usize,
    pub max_configuration_entries: usize,
    pub max_configuration_text_bytes: usize,
    pub max_mcp_servers: usize,
    pub max_mcp_arguments: usize,
    pub max_mcp_tool_overrides: usize,
    pub max_package_registries: usize,
}

impl EnvironmentLimits {
    pub const CURRENT: Self = Self {
        max_name_bytes: capsem_proto::ENVIRONMENT_MAX_NAME_BYTES,
        max_description_bytes: capsem_proto::ENVIRONMENT_MAX_DESCRIPTION_BYTES,
        max_startup_script_bytes: capsem_proto::ENVIRONMENT_MAX_STARTUP_SCRIPT_BYTES,
        max_variables: capsem_proto::ENVIRONMENT_MAX_VARIABLES,
        max_variable_name_bytes: capsem_proto::ENVIRONMENT_MAX_VARIABLE_NAME_BYTES,
        max_variable_value_bytes: capsem_proto::ENVIRONMENT_MAX_VARIABLE_VALUE_BYTES,
        max_variable_total_bytes: capsem_proto::ENVIRONMENT_MAX_VARIABLE_TOTAL_BYTES,
        max_secret_bindings: capsem_proto::ENVIRONMENT_MAX_VARIABLES,
        max_repositories: capsem_proto::ENVIRONMENT_MAX_REPOSITORIES,
        max_spec_bytes: capsem_proto::ENVIRONMENT_MAX_SPEC_BYTES,
        max_catalog_selections: capsem_proto::ENVIRONMENT_MAX_CATALOG_SELECTIONS,
        max_catalog_reference_bytes: capsem_proto::ENVIRONMENT_MAX_CATALOG_REFERENCE_BYTES,
        max_rules: capsem_proto::ENVIRONMENT_MAX_RULES,
        max_rule_bytes: capsem_proto::ENVIRONMENT_MAX_RULE_BYTES,
        max_configuration_entries: capsem_proto::ENVIRONMENT_MAX_CONFIGURATION_ENTRIES,
        max_configuration_text_bytes: capsem_proto::ENVIRONMENT_MAX_CONFIGURATION_TEXT_BYTES,
        max_mcp_servers: capsem_proto::ENVIRONMENT_MAX_MCP_SERVERS,
        max_mcp_arguments: capsem_proto::ENVIRONMENT_MAX_MCP_ARGUMENTS,
        max_mcp_tool_overrides: capsem_proto::ENVIRONMENT_MAX_MCP_TOOL_OVERRIDES,
        max_package_registries: capsem_proto::ENVIRONMENT_MAX_PACKAGE_REGISTRIES,
    };
}
