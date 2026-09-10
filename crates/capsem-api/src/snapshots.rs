use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotOrigin {
    Auto,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SnapshotInfo {
    pub checkpoint: String,
    pub slot: usize,
    pub origin: SnapshotOrigin,
    pub name: Option<String>,
    pub timestamp: String,
    pub hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SnapshotsStatus {
    pub total: usize,
    pub auto_count: usize,
    pub manual_count: usize,
    pub manual_available: usize,
    pub snapshots: Vec<SnapshotInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SnapshotsList {
    pub total: usize,
    pub snapshots: Vec<SnapshotInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ChangesQuery {
    /// Checkpoint from snapshots.list(), for example cp-0.
    pub checkpoint: String,
    /// Number of file changes to skip, in path order.
    #[serde(default)]
    pub offset: usize,
    /// Maximum returned changes. Defaults to 200, capped at 2000.
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FileChangeKind {
    Created,
    Modified,
    Deleted,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FileChange {
    pub path: String,
    pub kind: FileChangeKind,
    pub size: Option<u64>,
    pub is_symlink: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ChangesResponse {
    /// The requested baseline. Changes describe UTF-8 file paths and symlinks;
    /// directories are traversed, and symlink targets are never followed.
    /// A live workspace can change during the comparison.
    pub checkpoint: String,
    pub changes: Vec<FileChange>,
    pub total: usize,
    pub has_more: bool,
}
