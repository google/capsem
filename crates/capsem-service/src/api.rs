use capsem_core::session::{
    GlobalStats, McpToolSummary, ProviderSummary, SessionRecord, ToolSummary,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::registry::{SavedVmBaseAssets, SavedVmProfilePin};

/// Response for GET /stats -- full main.db dump in one call.
#[derive(Serialize, Debug)]
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
    /// RAM in megabytes. If absent, service resolves from merged VM settings
    /// (vm.resources.ram_gb, default 8 GiB).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ram_mb: Option<u64>,
    /// CPU count. If absent, service resolves from merged VM settings
    /// (vm.resources.cpu_count, default 4).
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
    /// Profile id to resolve for a fresh VM. Clones inherit the source VM's
    /// profile pin instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    /// Optional exact installed profile revision to require for a fresh VM.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_revision: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ForkRequest {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ForkResponse {
    pub name: String,
    pub size_bytes: u64,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ProvisionResponse {
    pub id: String,
    /// The UDS path the per-VM capsem-process is listening on. Clients MUST
    /// use this value rather than recomputing it -- the service may fall back
    /// to a short hashed path under /tmp/capsem/ when the preferred path
    /// would exceed SUN_LEN. See capsem_core::uds::instance_socket_path.
    #[serde(default)]
    pub uds_path: Option<std::path::PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_status: Option<VmProfileStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_pin: Option<SavedVmProfilePin>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_health: Option<AssetHealth>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SandboxInfo {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub pid: u32,
    pub status: String,
    #[serde(default)]
    pub persistent: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ram_mb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpus: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_assets: Option<SavedVmBaseAssets>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_pin: Option<SavedVmProfilePin>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forked_from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// On-disk size of the session dir in bytes. Populated for /info on
    /// persistent VMs; useful for verifying that fork produced a compact
    /// overlay and not a bloated sparse file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    // -- Telemetry (populated for /info, omitted when absent) --
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vm_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_status: Option<VmProfileStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
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
    pub total_mcp_calls: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_requests: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_requests: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub denied_requests: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_dns_queries: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub denied_dns_queries: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_file_events: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_event_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_exec_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_call_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security_events_total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enforcement_decisions_total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detection_findings_total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocks_total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_block_event_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_block_rule_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_block_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_detection_event_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_detection_rule_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_detection_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_detection_severity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics_schema_version: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics_captured_at_unix_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub configured_ram_mb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub configured_vcpus: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_process_rss_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_cpu_time_micros: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_cpu_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_disk_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_disk_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rootfs_overlay_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_queue_notifications_total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_queue_drains_total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_descriptors_drained_total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_used_entries_total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_interrupts_raised_total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_interrupts_suppressed_total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_read_ops_total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_write_ops_total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_bytes_read_total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_bytes_written_total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_async_submissions_total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_async_completions_total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_async_fallbacks_total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_async_queue_full_total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_async_in_flight: Option<u64>,
    /// Short tail of `process.log` from the last failed boot. Populated
    /// only when `status == "Defunct"`. Renders in `capsem list` /
    /// `capsem status` so a crashed VM tells the user *why* without
    /// requiring a separate `capsem logs <id>` round-trip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

impl SandboxInfo {
    /// Construct with only the core fields; all telemetry fields default to None.
    pub fn new(id: String, pid: u32, status: String, persistent: bool) -> Self {
        Self {
            id,
            name: None,
            pid,
            status,
            persistent,
            ram_mb: None,
            cpus: None,
            version: None,
            base_assets: None,
            profile_pin: None,
            forked_from: None,
            description: None,
            size_bytes: None,
            vm_id: None,
            profile_id: None,
            profile_revision: None,
            profile_status: None,
            user_id: None,
            created_at: None,
            uptime_secs: None,
            total_input_tokens: None,
            total_output_tokens: None,
            total_estimated_cost: None,
            total_tool_calls: None,
            total_mcp_calls: None,
            total_requests: None,
            allowed_requests: None,
            denied_requests: None,
            total_dns_queries: None,
            denied_dns_queries: None,
            total_file_events: None,
            process_event_count: None,
            process_exec_count: None,
            model_call_count: None,
            security_events_total: None,
            enforcement_decisions_total: None,
            detection_findings_total: None,
            blocks_total: None,
            latest_block_event_id: None,
            latest_block_rule_id: None,
            latest_block_reason: None,
            latest_detection_event_id: None,
            latest_detection_rule_id: None,
            latest_detection_title: None,
            latest_detection_severity: None,
            metrics_schema_version: None,
            metrics_captured_at_unix_ms: None,
            configured_ram_mb: None,
            configured_vcpus: None,
            host_pid: None,
            host_process_rss_bytes: None,
            host_cpu_time_micros: None,
            host_cpu_percent: None,
            session_disk_bytes: None,
            workspace_disk_bytes: None,
            rootfs_overlay_bytes: None,
            block_queue_notifications_total: None,
            block_queue_drains_total: None,
            block_descriptors_drained_total: None,
            block_used_entries_total: None,
            block_interrupts_raised_total: None,
            block_interrupts_suppressed_total: None,
            block_read_ops_total: None,
            block_write_ops_total: None,
            block_bytes_read_total: None,
            block_bytes_written_total: None,
            block_async_submissions_total: None,
            block_async_completions_total: None,
            block_async_fallbacks_total: None,
            block_async_queue_full_total: None,
            block_async_in_flight: None,
            last_error: None,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VmProfileStatus {
    Current,
    NeedsUpdate,
    Deprecated,
    Revoked,
    Corrupted,
    Unknown,
}

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    /// Profile id to resolve for the temporary VM.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    /// Optional exact installed profile revision to require for the temporary VM.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_revision: Option<String>,
    /// Guest RAM in MiB. Falls back to merged VM settings
    /// (vm.resources.ram_gb, default 8 GiB).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ram_mb: Option<u64>,
    /// Guest CPU count. Falls back to merged VM settings
    /// (vm.resources.cpu_count, default 4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpus: Option<u32>,
    /// Environment variables to inject into the guest at boot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AssetHealthState {
    Checking,
    Updating,
    Ready,
    Error,
}

impl AssetHealthState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Checking => "checking",
            Self::Updating => "updating",
            Self::Ready => "ready",
            Self::Error => "error",
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AssetProgress {
    pub logical_name: String,
    pub bytes_done: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_total: Option<u64>,
    pub done: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SavedVmAssetDependency {
    pub vm: String,
    pub asset_version: String,
    pub arch: String,
    pub missing: Vec<String>,
    pub recovery_hint: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ProfileAssetProvenance {
    pub logical_name: String,
    pub hash: String,
    pub source_url: String,
    pub size: u64,
    pub content_type: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AssetHealth {
    pub ready: bool,
    pub state: AssetHealthState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_payload_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profile_assets: Vec<ProfileAssetProvenance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
    pub missing: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<AssetProgress>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub retry_count: u32,
    #[serde(default)]
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub saved_vm_dependencies: Vec<SavedVmAssetDependency>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked_at_unix_secs: Option<u64>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ListResponse {
    pub sandboxes: Vec<SandboxInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_health: Option<AssetHealth>,
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

/// Response for GET /files/{id}.
#[derive(Serialize, Debug)]
pub struct FileListResponse {
    pub entries: Vec<FileListEntry>,
}

/// Response for POST /files/{id}/content (upload).
#[derive(Serialize, Deserialize, Debug)]
pub struct UploadResponse {
    pub success: bool,
    pub size: u64,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct LogsResponse {
    pub logs: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serial_logs: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_logs: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security_logs: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct InspectRequest {
    pub sql: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[allow(dead_code)]
pub struct InspectResponse {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<serde_json::Value>>,
}

/// Query parameters for GET /history/{id}.
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

/// Response for GET /history/{id}.
#[derive(Serialize, Debug)]
#[allow(dead_code)]
pub struct HistoryResponse {
    pub commands: Vec<capsem_logger::HistoryEntry>,
    pub total: u64,
    pub has_more: bool,
}

/// Response for GET /history/{id}/processes.
#[derive(Serialize, Debug)]
#[allow(dead_code)]
pub struct HistoryProcessesResponse {
    pub processes: Vec<capsem_logger::ProcessEntry>,
}

/// Response for GET /history/{id}/counts.
#[derive(Serialize, Debug)]
#[allow(dead_code)]
pub struct HistoryCountsResponse {
    pub exec_count: u64,
    pub audit_count: u64,
}

/// Query parameters for GET /history/{id}/transcript.
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

/// Response for GET /history/{id}/transcript.
#[derive(Serialize, Debug)]
#[allow(dead_code)]
pub struct TranscriptResponse {
    pub content: String,
    pub bytes: usize,
}

// ---------------------------------------------------------------------------
// Setup / Onboarding types
// ---------------------------------------------------------------------------

#[derive(Deserialize, Debug)]
pub struct ValidateKeyRequest {
    pub provider: String,
    pub key: String,
}

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
        let json = json!({"name": "my-vm", "ram_mb": 4096, "cpus": 4, "persistent": true});
        let r: ProvisionRequest = serde_json::from_value(json).unwrap();
        assert_eq!(r.name, Some("my-vm".into()));
        assert_eq!(r.ram_mb, Some(4096));
        assert_eq!(r.cpus, Some(4));
        assert!(r.persistent);
        assert!(r.env.is_none());
    }

    #[test]
    fn provision_request_ram_cpus_omitted_deserializes_as_none() {
        // Service handler fills these from merged VM settings. Callers like
        // the tray's "New Session" rely on this to honor user defaults.
        let json = json!({"name": "my-vm"});
        let r: ProvisionRequest = serde_json::from_value(json).unwrap();
        assert_eq!(r.ram_mb, None);
        assert_eq!(r.cpus, None);
    }

    #[test]
    fn provision_request_with_env() {
        let json = json!({"ram_mb": 2048, "cpus": 2, "env": {"FOO": "bar", "BAZ": "qux"}});
        let r: ProvisionRequest = serde_json::from_value(json).unwrap();
        let env = r.env.unwrap();
        assert_eq!(env.get("FOO").unwrap(), "bar");
        assert_eq!(env.get("BAZ").unwrap(), "qux");
    }

    #[test]
    fn provision_request_with_profile_selection() {
        let json = json!({
            "profile_id": "coding",
            "profile_revision": "2026.0520.1"
        });
        let r: ProvisionRequest = serde_json::from_value(json).unwrap();
        assert_eq!(r.profile_id.as_deref(), Some("coding"));
        assert_eq!(r.profile_revision.as_deref(), Some("2026.0520.1"));
    }

    #[test]
    fn provision_request_env_omitted() {
        let r = ProvisionRequest {
            name: None,
            ram_mb: Some(2048),
            cpus: Some(2),
            persistent: false,
            env: None,
            from: None,
            profile_id: None,
            profile_revision: None,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(!json.contains("env"));
        assert!(!json.contains("from"));
    }

    #[test]
    fn provision_request_without_name() {
        let json = json!({"ram_mb": 2048, "cpus": 2});
        let r: ProvisionRequest = serde_json::from_value(json).unwrap();
        assert_eq!(r.name, None);
        assert!(!r.persistent);
    }

    #[test]
    fn provision_request_with_from() {
        let json = json!({"ram_mb": 2048, "cpus": 2, "from": "my-fork"});
        let r: ProvisionRequest = serde_json::from_value(json).unwrap();
        assert_eq!(r.from.as_deref(), Some("my-fork"));
    }

    #[test]
    fn provision_request_image_alias_deserializes_to_from() {
        let json = json!({"ram_mb": 2048, "cpus": 2, "image": "old-img"});
        let r: ProvisionRequest = serde_json::from_value(json).unwrap();
        assert_eq!(r.from.as_deref(), Some("old-img"));
    }

    #[test]
    fn provision_response_roundtrip() {
        let r = ProvisionResponse {
            id: "vm-123".into(),
            uds_path: Some(std::path::PathBuf::from("/tmp/r/instances/vm-123.sock")),
            profile_id: Some("everyday-work".into()),
            profile_revision: Some("2026.0520.1".into()),
            profile_status: Some(VmProfileStatus::Current),
            profile_pin: None,
            asset_health: None,
        };
        let json = serde_json::to_string(&r).unwrap();
        let r2: ProvisionResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(r2.id, "vm-123");
        assert_eq!(
            r2.uds_path.as_deref(),
            Some(std::path::Path::new("/tmp/r/instances/vm-123.sock"))
        );
        assert_eq!(r2.profile_id.as_deref(), Some("everyday-work"));
        assert_eq!(r2.profile_revision.as_deref(), Some("2026.0520.1"));
        assert_eq!(r2.profile_status, Some(VmProfileStatus::Current));
    }

    // -----------------------------------------------------------------------
    // ListResponse
    // -----------------------------------------------------------------------

    #[test]
    fn list_response_empty() {
        let r = ListResponse {
            sandboxes: vec![],
            asset_health: None,
        };
        let json = serde_json::to_string(&r).unwrap();
        let r2: ListResponse = serde_json::from_str(&json).unwrap();
        assert!(r2.sandboxes.is_empty());
    }

    #[test]
    fn list_response_multiple() {
        let r = ListResponse {
            sandboxes: vec![
                {
                    let mut s = SandboxInfo::new("a".into(), 100, "Running".into(), true);
                    s.name = Some("a".into());
                    s.ram_mb = Some(2048);
                    s.cpus = Some(2);
                    s
                },
                SandboxInfo::new("b".into(), 200, "Running".into(), false),
            ],
            asset_health: None,
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
        let s = SandboxInfo::new("x".into(), 1, "Running".into(), false);
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("ram_mb"));
        assert!(!json.contains("cpus"));
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
        // ram_mb/cpus omitted -> None; handler resolves from VM settings.
        let json = json!({"command": "echo hello"});
        let r: RunRequest = serde_json::from_value(json).unwrap();
        assert_eq!(r.command, "echo hello");
        assert_eq!(r.timeout_secs, None);
        assert_eq!(r.profile_id, None);
        assert_eq!(r.profile_revision, None);
        assert_eq!(r.ram_mb, None);
        assert_eq!(r.cpus, None);
    }

    #[test]
    fn run_request_custom() {
        let json = json!({
            "command": "ls",
            "timeout_secs": 120,
            "profile_id": "coding",
            "profile_revision": "2026.0520.1",
            "ram_mb": 4096,
            "cpus": 4
        });
        let r: RunRequest = serde_json::from_value(json).unwrap();
        assert_eq!(r.timeout_secs, Some(120));
        assert_eq!(r.profile_id.as_deref(), Some("coding"));
        assert_eq!(r.profile_revision.as_deref(), Some("2026.0520.1"));
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
    // Files API
    // -----------------------------------------------------------------------

    #[test]
    fn upload_response_roundtrip() {
        let r = UploadResponse {
            success: true,
            size: 4,
        };
        let json = serde_json::to_string(&r).unwrap();
        let r2: UploadResponse = serde_json::from_str(&json).unwrap();
        assert!(r2.success);
        assert_eq!(r2.size, 4);
    }

    // -----------------------------------------------------------------------
    // Inspect
    // -----------------------------------------------------------------------

    #[test]
    fn inspect_request_roundtrip() {
        let json = json!({"sql": "SELECT count(*) FROM net_events"});
        let r: InspectRequest = serde_json::from_value(json).unwrap();
        assert_eq!(r.sql, "SELECT count(*) FROM net_events");
    }

    #[test]
    fn inspect_response_roundtrip() {
        let r = InspectResponse {
            columns: vec!["name".into(), "count".into()],
            rows: vec![vec![json!("net_events"), json!(42)]],
        };
        let json = serde_json::to_string(&r).unwrap();
        let r2: InspectResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(r2.columns.len(), 2);
        assert_eq!(r2.rows[0][1], json!(42));
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
            security_logs: None,
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
