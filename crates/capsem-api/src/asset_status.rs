//! Readiness and provenance of the VM boot assets, included in the hypervisor
//! overview and answered by `/assets/status`.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ValidationStatus {
    Valid,
    Invalid,
    Missing,
    FetchError,
}

/// Whether the one runtime asset set every new VM boots -- kernel, initrd and
/// rootfs of the installed manifest's current release -- is on disk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct AssetStatus {
    pub ready: bool,
    /// An asset reconciliation (startup or `/assets/ensure`) is running.
    pub downloading: bool,
    pub current_arch: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_version: Option<String>,
    pub assets: Vec<AssetFileStatus>,
    pub errors: Vec<String>,
    pub manifest: AssetManifestStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_asset: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_done: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_total: Option<u64>,
    /// Assets the last finished reconciliation downloaded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub downloaded: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reconcile_error: Option<String>,
    /// Set by `/assets/ensure`: whether this call started a reconciliation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct AssetFileStatus {
    /// `kernel`, `initrd` or `rootfs`.
    pub kind: String,
    /// Logical asset name in the manifest, e.g. `vmlinuz`.
    pub name: String,
    pub path: String,
    pub status: AssetFileState,
    pub expected_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_size: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AssetFileState {
    Present,
    Missing,
    /// On disk with a size the manifest does not record for it.
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
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
