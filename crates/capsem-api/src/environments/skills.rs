//! Read-only selected skill inventory and whole, digest-pinned UTF-8 content.
use super::*;
use capsem_config::catalogs::CatalogEntryMetadata;
use std::collections::BTreeMap;

#[derive(Debug, Deserialize, IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in = Query)]
pub struct EnvironmentSkillListQuery {
    /// Require this exact current saved revision.
    #[param(format = "uuid")]
    pub revision_id: Option<String>,
    /// Case-insensitive match on protected selection metadata.
    pub search: Option<String>,
    /// Continue with the cursor for the same saved revision.
    pub after: Option<String>,
    /// Number of matches per page, from 1 to 64. Defaults to 24.
    pub limit: Option<u16>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentSkillAvailability {
    Available,
    Revoked,
    Unavailable,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentSkillSummary {
    pub selection: EnvironmentCatalogPin,
    pub metadata: Option<CatalogEntryMetadata>,
    pub availability: EnvironmentSkillAvailability,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentSkillListResponse {
    pub revision: EnvironmentRevisionReference,
    pub skills: Vec<EnvironmentSkillSummary>,
    /// Whether the current catalog was checked. A verified retained pin may be available offline.
    pub discovery_fresh: bool,
    pub next_cursor: Option<String>,
}

impl std::fmt::Debug for EnvironmentSkillListResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EnvironmentSkillListResponse")
            .field("revision", &self.revision)
            .field("skill_count", &self.skills.len())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentSkillDetailResponse {
    pub revision: EnvironmentRevisionReference,
    pub selection: EnvironmentCatalogPin,
    pub metadata: CatalogEntryMetadata,
    pub discovery_fresh: bool,
    /// Complete UTF-8 content keyed by normalized relative path.
    pub files: BTreeMap<String, String>,
}

impl std::fmt::Debug for EnvironmentSkillDetailResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EnvironmentSkillDetailResponse")
            .field("revision", &self.revision)
            .field("skill_id", &self.selection.id)
            .field("file_count", &self.files.len())
            .finish_non_exhaustive()
    }
}

/// Applied selections for the exact acknowledged sandbox generation.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct VmSkillListResponse {
    pub vm_id: String,
    #[schema(format = "uuid")]
    pub owner_generation: String,
    pub revision: Option<EnvironmentRevisionReference>,
    pub skills: Vec<EnvironmentSkillSummary>,
    pub discovery_fresh: bool,
    pub next_cursor: Option<String>,
}

impl std::fmt::Debug for VmSkillListResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VmSkillListResponse")
            .field("vm_id", &self.vm_id)
            .field("owner_generation", &self.owner_generation)
            .field("revision", &self.revision)
            .field("skill_count", &self.skills.len())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct VmSkillDetailResponse {
    pub vm_id: String,
    #[schema(format = "uuid")]
    pub owner_generation: String,
    pub skill: EnvironmentSkillDetailResponse,
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in = Query)]
pub struct SkillCatalogListQuery {
    /// Case-insensitive match on catalog entry metadata.
    pub search: Option<String>,
    /// Continue with the cursor for the same source and catalog digest.
    pub after: Option<String>,
    /// Number of matches per page, from 1 to 64. Defaults to 24.
    pub limit: Option<u16>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in = Query)]
pub struct SkillCatalogDetailQuery {
    /// Exact published entry version.
    pub version: String,
    /// Source and digest token returned by the catalog list.
    pub snapshot: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillCatalogSnapshot {
    pub reference: String,
    pub digest: String,
    /// Supply this token when reading a skill from the snapshot.
    pub snapshot: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillCatalogListResponse {
    pub catalog: SkillCatalogSnapshot,
    pub skills: Vec<CatalogEntryMetadata>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillCatalogDetailResponse {
    pub selection: EnvironmentCatalogPin,
    pub metadata: CatalogEntryMetadata,
    /// Complete UTF-8 content keyed by normalized relative path.
    pub files: BTreeMap<String, String>,
}

impl std::fmt::Debug for SkillCatalogDetailResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SkillCatalogDetailResponse")
            .field("skill_id", &self.selection.id)
            .field("file_count", &self.files.len())
            .finish_non_exhaustive()
    }
}
