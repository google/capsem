use capsem_core::net::policy_config::{DetectionLevel, ProfileConfigFile, SecurityRuleAction};
use capsem_core::session::{
    GlobalStats, McpToolSummary, ProviderSummary, SessionRecord, ToolSummary,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::collections::HashMap;

/// Response for GET /stats -- global session stats from the logger DB boundary.
#[derive(Serialize, Debug, Clone)]
pub struct StatsResponse {
    pub global: GlobalStats,
    pub sessions: Vec<SessionRecord>,
    pub top_providers: Vec<ProviderSummary>,
    pub top_tools: Vec<ToolSummary>,
    pub top_mcp_tools: Vec<McpToolSummary>,
}

#[derive(Serialize, Deserialize, Debug)]
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

#[derive(Serialize, Deserialize, Debug)]
pub struct ForkRequest {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ForkResponse {
    pub id: String,
    pub name: String,
    pub size_bytes: u64,
}

#[derive(Serialize, Deserialize, Debug)]
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
    /// to a short hashed path under /tmp/capsem/ when the preferred path
    /// would exceed SUN_LEN. See capsem_core::uds::instance_socket_path.
    #[serde(default)]
    pub uds_path: Option<std::path::PathBuf>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmLifecycleState {
    Running,
    Stopped,
    Suspended,
    Defunct,
    Incompatible,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VmAction {
    Pause,
    Stop,
    Start,
    Resume,
    Fork,
    Delete,
}

impl VmLifecycleState {
    pub fn available_actions(self, can_resume: bool) -> Vec<VmAction> {
        match self {
            Self::Running => vec![
                VmAction::Pause,
                VmAction::Stop,
                VmAction::Fork,
                VmAction::Delete,
            ],
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

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
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

#[derive(Serialize, Deserialize, Debug)]
pub struct SessionDbStatus {
    pub ready: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
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
    // -- Telemetry (populated for /info, omitted when absent) --
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uptime_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_input_tokens: Option<u64>,
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
    pub fn new(
        id: String,
        profile_id: String,
        pid: u32,
        status: VmLifecycleState,
        persistent: bool,
    ) -> Self {
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
            created_at: None,
            uptime_secs: None,
            total_input_tokens: None,
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

#[derive(Serialize, Deserialize, Debug)]
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

#[derive(Deserialize, Debug, Default)]
pub struct VmEditRequest {
    #[serde(default)]
    pub ram_mb: Option<u64>,
    #[serde(default)]
    pub cpus: Option<u32>,
    #[serde(default)]
    pub persistent: Option<bool>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub profile_id: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct VmOperationStatusResponse {
    pub vm_id: String,
    pub operation: String,
    pub status: String,
    pub in_progress: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct UpdateStatusResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_url: Option<String>,
    pub stale: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub binary: UpdateTrackStatus,
    pub assets: UpdateTrackStatus,
    pub profiles: UpdateTrackStatus,
    pub images: UpdateTrackStatus,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct UpdateTrackStatus {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest: Option<String>,
    pub update_available: bool,
    pub state: UpdateTrackState,
    pub compatibility: UpdateCompatibilityState,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UpdateTrackState {
    Current,
    UpdateAvailable,
    Unknown,
    NotPublished,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UpdateCompatibilityState {
    Compatible,
    Unknown,
    NotApplicable,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ProfileSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_svg: Option<String>,
    pub availability: ProfileAvailabilitySummary,
    pub source: String,
    pub rule_count: usize,
    pub default_rule_count: usize,
    pub plugin_count: usize,
    pub mcp_server_count: usize,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ProfileAvailabilitySummary {
    pub web: bool,
    pub shell: bool,
    pub mobile: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ProfilesListResponse {
    pub profiles: Vec<ProfileSummary>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ProfileInfoResponse {
    pub profile: ProfileSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub obom: Option<ProfileObomInfo>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ProfileObomInfo {
    pub profile_id: String,
    pub current_arch: String,
    pub scope: String,
    pub format: String,
    pub name: String,
    pub url: String,
    pub hash: String,
    pub size: u64,
    pub generator: String,
    pub generator_version: String,
    pub rootfs_hash: String,
    pub route: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ProfileObomResponse {
    pub profile_id: String,
    pub current_arch: String,
    pub obom: ProfileObomInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document: Option<serde_json::Value>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ProfileValidateRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toml: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<ProfileConfigFile>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ProfileValidateResponse {
    pub valid: bool,
    pub profile_id: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EnforcementRuleSource {
    BuiltinDefault,
    Profile,
    Corp,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct EnforcementRuleInfo {
    pub rule_id: String,
    pub source: EnforcementRuleSource,
    pub provider: String,
    pub namespace: String,
    pub rule_key: String,
    pub default_rule: bool,
    pub enabled: bool,
    pub name: String,
    pub action: SecurityRuleAction,
    #[serde(rename = "match")]
    pub condition: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detection_level: Option<DetectionLevel>,
    pub priority: i32,
    pub corp_locked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct EnforcementRuleListResponse {
    pub profile_id: String,
    pub rules: Vec<EnforcementRuleInfo>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct EnforcementInfoResponse {
    pub profile_id: String,
    pub rule_count: usize,
    pub default_rule_count: usize,
    pub custom_rule_count: usize,
    pub detection_rule_count: usize,
    pub corp_locked_rule_count: usize,
    pub source_counts: BTreeMap<String, usize>,
    pub action_counts: BTreeMap<String, usize>,
}

pub type DetectionRuleInfo = EnforcementRuleInfo;
pub type DetectionRuleListResponse = EnforcementRuleListResponse;
pub type DetectionInfoResponse = EnforcementInfoResponse;

#[derive(Serialize, Deserialize, Debug)]
pub struct PersistRequest {
    pub name: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct PurgeRequest {
    #[serde(default)]
    pub all: bool,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct PurgeResponse {
    pub purged: u32,
    pub persistent_purged: u32,
    pub ephemeral_purged: u32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RunRequest {
    pub command: String,
    pub profile_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    /// Guest RAM in MiB. Falls back to the selected profile's VM resources.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ram_mb: Option<u64>,
    /// Guest CPU count. Falls back to the selected profile's VM resources.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpus: Option<u32>,
    /// Environment variables to inject into the guest at boot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ListResponse {
    pub sandboxes: Vec<SandboxInfo>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ExecRequest {
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ExecResponse {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct WriteFileRequest {
    pub path: String,
    pub content: String, // Base64 or plain text? For now let's assume plain text or base64 if we detect it.
}

// ── Files API types (host-side VirtioFS) ─────────────────────────────

/// A single entry in a file listing.
#[derive(Serialize, Debug, Clone)]
pub struct FileListEntry {
    pub name: String,
    pub path: String,
    #[serde(rename = "type")]
    pub entry_type: String,
    pub size: u64,
    pub mtime: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_text: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<FileListEntry>>,
}

/// Response for GET /vms/{id}/files/list.
#[derive(Serialize, Debug)]
pub struct FileListResponse {
    pub entries: Vec<FileListEntry>,
}

/// Response for POST /vms/{id}/files/content (upload).
#[derive(Serialize, Debug)]
pub struct UploadResponse {
    pub success: bool,
    pub size: u64,
}

// ── Legacy vsock file I/O types ──────────────────────────────────────

#[derive(Serialize, Deserialize, Debug)]
pub struct ReadFileRequest {
    pub path: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ReadFileResponse {
    pub content: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct LogsResponse {
    pub logs: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serial_logs: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_logs: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ErrorResponse {
    pub error: String,
}

// ── MCP API types ──────────────────────────────────────────────────

/// Response for GET /profiles/{profile_id}/mcp/servers/list.
#[derive(Serialize, Deserialize, Debug)]
pub struct McpServerInfoResponse {
    pub name: String,
    pub url: String,
    pub has_auth_credential: bool,
    pub custom_header_count: usize,
    pub source: String,
    pub enabled: bool,
    pub running: bool,
    pub tool_count: usize,
    pub is_stdio: bool,
}

/// Response for GET /profiles/{profile_id}/mcp/default/info.
#[derive(Serialize, Deserialize, Debug)]
pub struct McpDefaultPermissionResponse {
    pub action: capsem_core::net::policy_config::SecurityRuleAction,
    pub source: String,
    pub rule_id: Option<String>,
}

/// Response for GET /profiles/{profile_id}/mcp/servers/{server_id}/tools/list.
#[derive(Serialize, Deserialize, Debug)]
pub struct McpToolInfoResponse {
    pub namespaced_name: String,
    pub original_name: String,
    pub description: Option<String>,
    pub server_name: String,
    pub annotations: Option<serde_json::Value>,
    pub pin_hash: Option<String>,
    pub pin_changed: bool,
    pub permission_action: capsem_core::net::policy_config::SecurityRuleAction,
    pub permission_source: String,
}

/// Query parameters for GET /vms/{id}/history.
#[derive(Deserialize, Debug)]
#[allow(dead_code)]
pub struct HistoryQuery {
    #[serde(default = "default_history_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
    pub search: Option<String>,
    #[serde(default = "default_history_layer")]
    pub layer: String,
}

#[allow(dead_code)]
fn default_history_limit() -> usize {
    500
}
#[allow(dead_code)]
fn default_history_layer() -> String {
    "all".to_string()
}

/// Response for GET /vms/{id}/history.
#[derive(Serialize, Debug)]
#[allow(dead_code)]
pub struct HistoryResponse {
    pub commands: Vec<capsem_logger::HistoryEntry>,
    pub total: u64,
    pub has_more: bool,
}

/// Response for GET /vms/{id}/history/processes.
#[derive(Serialize, Debug)]
#[allow(dead_code)]
pub struct HistoryProcessesResponse {
    pub processes: Vec<capsem_logger::ProcessEntry>,
}

/// Response for GET /vms/{id}/history/counts.
#[derive(Serialize, Debug)]
#[allow(dead_code)]
pub struct HistoryCountsResponse {
    pub exec_count: u64,
    pub audit_count: u64,
}

/// Query parameters for GET /vms/{id}/history/transcript.
#[derive(Deserialize, Debug)]
#[allow(dead_code)]
pub struct TranscriptQuery {
    #[serde(default = "default_tail_lines")]
    pub tail_lines: usize,
}

#[allow(dead_code)]
fn default_tail_lines() -> usize {
    500
}

/// Response for GET /vms/{id}/history/transcript.
#[derive(Serialize, Debug)]
#[allow(dead_code)]
pub struct TranscriptResponse {
    pub content: String,
    pub bytes: usize,
}

// ---------------------------------------------------------------------------
// Corporate configuration request types
// ---------------------------------------------------------------------------

#[derive(Deserialize, Debug)]
pub struct CorpConfigRequest {
    /// URL to fetch corp config from (e.g. https://corp.example.com/capsem.toml)
    pub source: Option<String>,
    /// Inline TOML content
    pub toml: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // -----------------------------------------------------------------------
    // ProvisionRequest / ProvisionResponse
    // -----------------------------------------------------------------------

    #[test]
    fn provision_request_with_name() {
        let json = json!({"name": "my-vm", "profile_id": "code", "ram_mb": 4096, "cpus": 4, "persistent": true});
        let r: ProvisionRequest = serde_json::from_value(json).unwrap();
        assert_eq!(r.name, Some("my-vm".into()));
        assert_eq!(r.profile_id, "code");
        assert_eq!(r.ram_mb, Some(4096));
        assert_eq!(r.cpus, Some(4));
        assert!(r.persistent);
        assert!(r.env.is_none());
    }

    #[test]
    fn provision_request_requires_profile_id() {
        let json = json!({"name": "my-vm", "ram_mb": 4096, "cpus": 4});
        let err = serde_json::from_value::<ProvisionRequest>(json).unwrap_err();
        assert!(err.to_string().contains("profile_id"));
    }

    #[test]
    fn provision_request_ram_cpus_omitted_deserializes_as_none() {
        // Service handler fills these from the selected profile. Callers like
        // the tray's "New Session" do not have to duplicate profile resources.
        let json = json!({"name": "my-vm", "profile_id": "code"});
        let r: ProvisionRequest = serde_json::from_value(json).unwrap();
        assert_eq!(r.ram_mb, None);
        assert_eq!(r.cpus, None);
    }

    #[test]
    fn provision_request_with_env() {
        let json = json!({"profile_id": "code", "ram_mb": 2048, "cpus": 2, "env": {"FOO": "bar", "BAZ": "qux"}});
        let r: ProvisionRequest = serde_json::from_value(json).unwrap();
        let env = r.env.unwrap();
        assert_eq!(env.get("FOO").unwrap(), "bar");
        assert_eq!(env.get("BAZ").unwrap(), "qux");
    }

    #[test]
    fn provision_request_env_omitted() {
        let r = ProvisionRequest {
            name: None,
            profile_id: "code".into(),
            ram_mb: Some(2048),
            cpus: Some(2),
            persistent: false,
            env: None,
            from: None,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(!json.contains("env"));
        assert!(!json.contains("from"));
    }

    #[test]
    fn provision_request_without_name() {
        let json = json!({"profile_id": "code", "ram_mb": 2048, "cpus": 2});
        let r: ProvisionRequest = serde_json::from_value(json).unwrap();
        assert_eq!(r.name, None);
        assert!(!r.persistent);
    }

    #[test]
    fn provision_request_with_from() {
        let json = json!({"profile_id": "code", "ram_mb": 2048, "cpus": 2, "from": "my-fork"});
        let r: ProvisionRequest = serde_json::from_value(json).unwrap();
        assert_eq!(r.from.as_deref(), Some("my-fork"));
    }

    #[test]
    fn provision_request_image_alias_deserializes_to_from() {
        let json = json!({"profile_id": "code", "ram_mb": 2048, "cpus": 2, "image": "old-img"});
        let r: ProvisionRequest = serde_json::from_value(json).unwrap();
        assert_eq!(r.from.as_deref(), Some("old-img"));
    }

    #[test]
    fn provision_response_roundtrip() {
        let r = ProvisionResponse {
            id: "vm-123".into(),
            name: "co-work1".into(),
            profile_id: "code".into(),
            status: VmLifecycleState::Running,
            persistent: true,
            can_resume: false,
            available_actions: vec![
                VmAction::Pause,
                VmAction::Stop,
                VmAction::Fork,
                VmAction::Delete,
            ],
            uds_path: Some(std::path::PathBuf::from("/tmp/r/instances/vm-123.sock")),
        };
        let json = serde_json::to_string(&r).unwrap();
        let r2: ProvisionResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(r2.id, "vm-123");
        assert_eq!(r2.name, "co-work1");
        assert_eq!(r2.profile_id, "code");
        assert_eq!(r2.status, VmLifecycleState::Running);
        assert!(r2.persistent);
        assert!(!r2.can_resume);
        assert_eq!(
            r2.available_actions,
            vec![
                VmAction::Pause,
                VmAction::Stop,
                VmAction::Fork,
                VmAction::Delete
            ]
        );
        assert_eq!(
            r2.uds_path.as_deref(),
            Some(std::path::Path::new("/tmp/r/instances/vm-123.sock"))
        );
    }

    // -----------------------------------------------------------------------
    // ListResponse
    // -----------------------------------------------------------------------

    #[test]
    fn list_response_empty() {
        let r = ListResponse { sandboxes: vec![] };
        let json = serde_json::to_string(&r).unwrap();
        let r2: ListResponse = serde_json::from_str(&json).unwrap();
        assert!(r2.sandboxes.is_empty());
    }

    #[test]
    fn list_response_multiple() {
        let r = ListResponse {
            sandboxes: vec![
                {
                    let mut s = SandboxInfo::new(
                        "a".into(),
                        "code".into(),
                        100,
                        VmLifecycleState::Running,
                        true,
                    );
                    s.name = Some("a".into());
                    s.ram_mb = Some(2048);
                    s.cpus = Some(2);
                    s
                },
                SandboxInfo::new(
                    "b".into(),
                    "code".into(),
                    200,
                    VmLifecycleState::Running,
                    false,
                ),
            ],
        };
        let json = serde_json::to_string(&r).unwrap();
        let r2: ListResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(r2.sandboxes.len(), 2);
        assert_eq!(r2.sandboxes[0].id, "a");
        assert!(r2.sandboxes[0].persistent);
        assert_eq!(r2.sandboxes[1].id, "b");
        assert!(!r2.sandboxes[1].persistent);
    }

    #[test]
    fn sandbox_info_optional_fields_omitted() {
        let s = SandboxInfo::new(
            "x".into(),
            "code".into(),
            1,
            VmLifecycleState::Running,
            false,
        );
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("ram_mb"));
        assert!(!json.contains("cpus"));
    }

    #[test]
    fn sandbox_info_rejects_unknown_lifecycle_state() {
        let json =
            r#"{"id":"x","profile_id":"code","pid":1,"status":"HalfRestored","persistent":true}"#;
        let err = serde_json::from_str::<SandboxInfo>(json).unwrap_err();
        assert!(err.to_string().contains("unknown variant"));
    }

    // -----------------------------------------------------------------------
    // PersistRequest / PurgeRequest / PurgeResponse
    // -----------------------------------------------------------------------

    #[test]
    fn persist_request_roundtrip() {
        let json = json!({"name": "mydev"});
        let r: PersistRequest = serde_json::from_value(json).unwrap();
        assert_eq!(r.name, "mydev");
    }

    #[test]
    fn purge_request_defaults() {
        let json = json!({});
        let r: PurgeRequest = serde_json::from_value(json).unwrap();
        assert!(!r.all);
    }

    #[test]
    fn purge_request_all() {
        let json = json!({"all": true});
        let r: PurgeRequest = serde_json::from_value(json).unwrap();
        assert!(r.all);
    }

    #[test]
    fn purge_response_roundtrip() {
        let r = PurgeResponse {
            purged: 5,
            persistent_purged: 2,
            ephemeral_purged: 3,
        };
        let json = serde_json::to_string(&r).unwrap();
        let r2: PurgeResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(r2.purged, 5);
        assert_eq!(r2.persistent_purged, 2);
        assert_eq!(r2.ephemeral_purged, 3);
    }

    // -----------------------------------------------------------------------
    // RunRequest
    // -----------------------------------------------------------------------

    #[test]
    fn run_request_defaults() {
        // ram_mb/cpus omitted -> None; handler resolves from the profile.
        let json = json!({"command": "echo hello", "profile_id": "code"});
        let r: RunRequest = serde_json::from_value(json).unwrap();
        assert_eq!(r.command, "echo hello");
        assert_eq!(r.profile_id, "code");
        assert_eq!(r.timeout_secs, None);
        assert_eq!(r.ram_mb, None);
        assert_eq!(r.cpus, None);
    }

    #[test]
    fn run_request_requires_profile_id() {
        let json = json!({"command": "echo hello"});
        let err = serde_json::from_value::<RunRequest>(json).unwrap_err();
        assert!(err.to_string().contains("profile_id"));
    }

    #[test]
    fn run_request_custom() {
        let json = json!({"command": "ls", "profile_id": "code", "timeout_secs": 120, "ram_mb": 4096, "cpus": 4});
        let r: RunRequest = serde_json::from_value(json).unwrap();
        assert_eq!(r.timeout_secs, Some(120));
        assert_eq!(r.ram_mb, Some(4096));
        assert_eq!(r.cpus, Some(4));
    }

    // -----------------------------------------------------------------------
    // ExecRequest / ExecResponse
    // -----------------------------------------------------------------------

    #[test]
    fn exec_request_defaults_to_no_timeout() {
        let json = json!({"command": "echo hi"});
        let r: ExecRequest = serde_json::from_value(json).unwrap();
        assert_eq!(r.command, "echo hi");
        assert_eq!(r.timeout_secs, None);
    }

    #[test]
    fn exec_request_custom_timeout() {
        let json = json!({"command": "sleep 10", "timeout_secs": 5});
        let r: ExecRequest = serde_json::from_value(json).unwrap();
        assert_eq!(r.timeout_secs, Some(5));
    }

    #[test]
    fn exec_response_roundtrip() {
        let r = ExecResponse {
            stdout: "hello\n".into(),
            stderr: "".into(),
            exit_code: 0,
        };
        let json = serde_json::to_string(&r).unwrap();
        let r2: ExecResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(r2.stdout, "hello\n");
        assert_eq!(r2.exit_code, 0);
    }

    // -----------------------------------------------------------------------
    // File I/O
    // -----------------------------------------------------------------------

    #[test]
    fn write_file_request_roundtrip() {
        let json = json!({"path": "/tmp/f.txt", "content": "data"});
        let r: WriteFileRequest = serde_json::from_value(json).unwrap();
        assert_eq!(r.path, "/tmp/f.txt");
        assert_eq!(r.content, "data");
    }

    #[test]
    fn read_file_response_roundtrip() {
        let r = ReadFileResponse {
            content: "file contents".into(),
        };
        let json = serde_json::to_string(&r).unwrap();
        let r2: ReadFileResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(r2.content, "file contents");
    }

    // -----------------------------------------------------------------------
    // Logs / Error
    // -----------------------------------------------------------------------

    #[test]
    fn logs_response_roundtrip() {
        let r = LogsResponse {
            logs: "Linux boot...\n".into(),
            serial_logs: None,
            process_logs: None,
        };
        let json = serde_json::to_string(&r).unwrap();
        let r2: LogsResponse = serde_json::from_str(&json).unwrap();
        assert!(r2.logs.contains("Linux"));
    }

    #[test]
    fn error_response_roundtrip() {
        let r = ErrorResponse {
            error: "sandbox not found".into(),
        };
        let json = serde_json::to_string(&r).unwrap();
        let r2: ErrorResponse = serde_json::from_str(&json).unwrap();
        assert!(r2.error.contains("not found"));
    }
}
