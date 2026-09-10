use crate::ValidationStatus;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum UpdateActionStatus {
    Planned,
    Succeeded,
    Failed,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
pub struct UpdateStatusResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation_status: Option<ValidationStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation_error: Option<String>,
    pub stale: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub binary: UpdateTrackStatus,
    pub assets: UpdateTrackStatus,
    pub profiles: UpdateTrackStatus,
    pub images: UpdateTrackStatus,
    pub supply_chain: SupplyChainEvidence,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateCheckRequest {
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateApplyRequest {
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub confirmed: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
pub struct UpdateCommandPlan {
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
pub struct UpdateActionResponse {
    pub status: UpdateActionStatus,
    pub command: UpdateCommandPlan,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default, ToSchema)]
pub struct SupplyChainEvidence {
    pub manifest: SupplyChainManifestEvidence,
    pub channel_index: SupplyChainChannelEvidence,
    pub host_sbom: SupplyChainReference,
    pub vm_obom: SupplyChainReference,
    pub attestations: Vec<SupplyChainReference>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default, ToSchema)]
pub struct SupplyChainManifestEvidence {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blake3: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default, ToSchema)]
pub struct SupplyChainChannelEvidence {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default, ToSchema)]
pub struct SupplyChainReference {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_artifact: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
pub struct UpdateTrackStatus {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    pub update_available: bool,
    pub state: UpdateTrackState,
    pub compatibility: UpdateCompatibilityState,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum UpdateTrackState {
    Current,
    UpdateAvailable,
    Unknown,
    NotPublished,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum UpdateCompatibilityState {
    Compatible,
    Unknown,
    NotApplicable,
}
