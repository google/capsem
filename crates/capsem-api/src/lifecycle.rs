use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct ProvisionRequest {
    pub name: Option<String>,
    pub profile_id: String,
    /// RAM in megabytes. If absent, service resolves from the selected
    /// profile's VM resources.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ram_mb: Option<u64>,
    /// CPU count. If absent, service resolves from the selected profile's VM
    /// resources.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpus: Option<u32>,
    /// When true, the VM is persistent (named VMs). Ephemeral VMs are destroyed on stop.
    #[serde(default)]
    pub persistent: bool,
    /// Environment variables to inject into the guest at boot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,
    /// Sandbox to clone state from. If provided, the new sandbox's session will
    /// be cloned from this existing persistent sandbox.
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "image")]
    pub from: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct ForkRequest {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct ForkResponse {
    pub id: String,
    pub name: String,
    pub size_bytes: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct ProvisionResponse {
    pub id: String,
    pub name: String,
    pub profile_id: String,
    pub status: VmLifecycleState,
    #[serde(default)]
    pub persistent: bool,
    #[serde(default)]
    pub can_resume: bool,
    pub available_actions: Vec<VmAction>,
    /// The UDS path the per-VM capsem-process is listening on. Clients MUST
    /// use this value rather than recomputing it -- the service may fall back
    /// to a short hashed path under /tmp/capsem-<uid>/ when the preferred path
    /// would exceed SUN_LEN. See capsem_foundation::uds::instance_socket_path.
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub uds_path: Option<std::path::PathBuf>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, ToSchema)]
pub enum VmLifecycleState {
    Running,
    Stopped,
    Suspended,
    Defunct,
    Incompatible,
}

impl VmLifecycleState {
    pub fn available_actions(self, can_resume: bool) -> Vec<VmAction> {
        match self {
            Self::Running => vec![VmAction::Pause, VmAction::Stop, VmAction::Fork, VmAction::Delete],
            Self::Stopped => {
                if can_resume {
                    vec![VmAction::Start, VmAction::Fork, VmAction::Delete]
                } else {
                    vec![VmAction::Fork, VmAction::Delete]
                }
            }
            Self::Suspended => {
                if can_resume {
                    vec![VmAction::Resume, VmAction::Fork, VmAction::Delete]
                } else {
                    vec![VmAction::Fork, VmAction::Delete]
                }
            }
            Self::Defunct | Self::Incompatible => vec![VmAction::Delete],
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum VmAction {
    Pause,
    Stop,
    Start,
    Resume,
    Fork,
    Delete,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
pub struct StorageDiagnostics {
    pub rootfs_image_path: String,
    pub rootfs_image_logical_bytes: u64,
    pub rootfs_image_physical_bytes: u64,
    pub host_total_bytes: u64,
    pub host_free_bytes: u64,
    pub host_available_bytes: u64,
    pub guest_overlay_device: String,
    pub guest_overlay_mount: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct SessionDbStatus {
    pub ready: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct SandboxInfo {
    pub id: String,
    pub profile_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub pid: u32,
    pub status: VmLifecycleState,
    #[serde(default)]
    pub persistent: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ram_mb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpus: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forked_from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// On-disk size of the session dir in bytes. Populated for /info on
    /// persistent VMs; useful for verifying that fork produced a compact
    /// overlay and not a bloated sparse file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage: Option<StorageDiagnostics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_db: Option<SessionDbStatus>,
    /// Session summaries returned by /info when the ledger is ready.
    /// Missing summaries are accompanied by session_db readiness/error details.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai: Option<crate::VmAiInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<crate::VmNetworkInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<crate::VmFilesInfo>,
    // -- Telemetry (populated by explicit stats/status aggregation surfaces,
    // omitted from hot lifecycle routes such as /vms/{id}/info) --
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uptime_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_thinking_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_estimated_cost: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tool_calls: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_requests: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_requests: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub denied_requests: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_file_events: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_call_count: Option<u64>,
    /// Short tail of `process.log` from the last failed boot. Populated
    /// only when `status == VmLifecycleState::Defunct`. Renders in `capsem list` /
    /// `capsem status` so a crashed VM tells the user *why* without
    /// requiring a separate `capsem logs <id>` round-trip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// True only when an inactive persistent VM can be started/resumed with
    /// the currently installed profile and pinned assets.
    #[serde(default)]
    pub can_resume: bool,
    /// Human-readable reason `can_resume` is false for an inactive persistent
    /// VM, e.g. profile payload hash drift after an upgrade.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume_blocked_reason: Option<String>,
    pub available_actions: Vec<VmAction>,
}

impl SandboxInfo {
    /// Construct with only the core fields; all telemetry fields default to None.
    pub fn new(id: String, profile_id: String, pid: u32, status: VmLifecycleState, persistent: bool) -> Self {
        let available_actions = status.available_actions(false);
        Self {
            id,
            profile_id,
            name: None,
            pid,
            status,
            persistent,
            ram_mb: None,
            cpus: None,
            version: None,
            forked_from: None,
            description: None,
            size_bytes: None,
            storage: None,
            session_db: None,
            ai: None,
            network: None,
            files: None,
            created_at: None,
            uptime_secs: None,
            total_input_tokens: None,
            total_thinking_tokens: None,
            total_output_tokens: None,
            total_estimated_cost: None,
            total_tool_calls: None,
            total_requests: None,
            allowed_requests: None,
            denied_requests: None,
            total_file_events: None,
            model_call_count: None,
            last_error: None,
            can_resume: false,
            resume_blocked_reason: None,
            available_actions,
        }
    }

    pub fn refresh_available_actions(&mut self) {
        self.available_actions = self.status.available_actions(self.can_resume);
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct VmStatusResponse {
    pub id: String,
    pub name: String,
    pub status: VmLifecycleState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default)]
    pub persistent: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uptime_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default)]
    pub can_resume: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume_blocked_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage: Option<StorageDiagnostics>,
    pub available_actions: Vec<VmAction>,
}
