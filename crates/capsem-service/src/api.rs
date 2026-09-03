use capsem_core::net::policy_config::{DetectionLevel, ProfileConfigFile, SecurityRuleAction};
use capsem_core::session::{GlobalStats, McpToolSummary, ProviderSummary, SessionRecord, ToolSummary};
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

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct VmStatsSummaryResponse {
    pub total_requests: u64,
    pub allowed_requests: u64,
    pub denied_requests: u64,
    pub total_input_tokens: u64,
    pub total_thinking_tokens: u64,
    pub total_output_tokens: u64,
    pub total_tool_calls: u64,
    pub total_estimated_cost: f64,
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
    /// would exceed SUN_LEN. See capsem_foundation::uds::instance_socket_path.
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation_status: Option<String>,
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

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SystemStatusResponse {
    pub version: String,
    pub service: String,
    pub manifest: serde_json::Value,
    pub manifest_metadata: serde_json::Value,
    pub profiles: serde_json::Value,
    pub corp: serde_json::Value,
    pub updates: UpdateStatusResponse,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct UpdateCheckRequest {
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct UpdateApplyRequest {
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub confirmed: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct UpdateCommandPlan {
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct UpdateActionResponse {
    pub status: String,
    pub command: UpdateCommandPlan,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
pub struct SupplyChainEvidence {
    pub manifest: SupplyChainManifestEvidence,
    pub channel_index: SupplyChainChannelEvidence,
    pub host_sbom: SupplyChainReference,
    pub vm_obom: SupplyChainReference,
    pub attestations: Vec<SupplyChainReference>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
pub struct SupplyChainManifestEvidence {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blake3: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
pub struct SupplyChainChannelEvidence {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
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

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
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
    pub update_semantics: ProfileUpdateSemantics,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ProfileUpdateSemantics {
    pub new_sessions: ProfileNewSessionUpdateSemantics,
    pub existing_vms: ProfileExistingVmUpdateSemantics,
    pub upgrade_action: ProfileUpgradeAction,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProfileNewSessionUpdateSemantics {
    UseCurrentProfileCatalog,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProfileExistingVmUpdateSemantics {
    PinnedUntilRecreate,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProfileUpgradeAction {
    RecreateVm,
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
    /// The guest produced more output than the per-exec cap allows, so
    /// `stdout` is a prefix. Defaulted so an older client still decodes.
    #[serde(default)]
    pub truncated: bool,
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
#[derive(Clone, Serialize, Deserialize, Debug)]
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
mod tests;
