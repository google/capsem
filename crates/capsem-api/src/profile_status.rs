//! Typed profile readiness and provenance included in the hypervisor overview.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::ProfileUpdateSemantics;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProfileCatalogSource {
    BuiltIn,
    Profile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ValidationStatus {
    Valid,
    Invalid,
    Missing,
    FetchError,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProfileCatalogStatus {
    pub source: ProfileCatalogSource,
    pub profile_count: usize,
    pub ready_count: usize,
    pub profiles: Vec<ProfileReadiness>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_manifest: Option<AssetManifestStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_asset: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_done: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub downloaded: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reconcile_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProfileReadiness {
    pub id: String,
    pub name: String,
    pub description: String,
    pub ready: bool,
    pub current_arch: String,
    pub missing_assets: Vec<ProfileArtifactIssue>,
    pub invalid_assets: Vec<ProfileArtifactIssue>,
    pub invalid_files: Vec<ProfileArtifactIssue>,
    pub errors: Vec<String>,
    pub asset_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_payload_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_semantics: Option<ProfileUpdateSemantics>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProfileArtifactIssue {
    /// Profile-owned artifact label, not a lifecycle or permission state.
    pub kind: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub present: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AssetManifestStatus {
    /// Provenance label recorded by the installer or channel metadata.
    pub origin: String,
    pub path: String,
    pub validation_status: ValidationStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blake3: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refreshed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packaged_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_policy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assets_current: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binaries_current: Option<String>,
}
