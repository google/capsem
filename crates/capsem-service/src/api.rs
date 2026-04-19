use std::collections::HashMap;
use capsem_core::session::{
    GlobalStats, McpToolSummary, ProviderSummary, SessionRecord, ToolSummary,
};
use serde::{Deserialize, Serialize};

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
    /// (vm.resources.ram_gb, default 4 GiB).
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
    pub total_file_events: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_call_count: Option<u64>,
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
            forked_from: None,
            description: None,
            size_bytes: None,
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
            total_file_events: None,
            model_call_count: None,
        }
    }
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
    #[serde(default = "default_run_timeout")]
    pub timeout_secs: u64,
    /// Guest RAM in MiB. Falls back to merged VM settings
    /// (vm.resources.ram_gb, default 4 GiB).
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

fn default_run_timeout() -> u64 { 60 }

#[derive(Serialize, Deserialize, Debug)]
pub struct AssetHealth {
    pub ready: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub missing: Vec<String>,
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
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

fn default_timeout() -> u64 { 30 }

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

/// Response for GET /files/{id}.
#[derive(Serialize, Debug)]
pub struct FileListResponse {
    pub entries: Vec<FileListEntry>,
}

/// Response for POST /files/{id}/content (upload).
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

/// Response for GET /mcp/servers.
#[derive(Serialize, Deserialize, Debug)]
pub struct McpServerInfoResponse {
    pub name: String,
    pub url: String,
    pub has_bearer_token: bool,
    pub custom_header_count: usize,
    pub source: String,
    pub enabled: bool,
    pub running: bool,
    pub tool_count: usize,
    pub is_stdio: bool,
}

/// Response for GET /mcp/tools.
#[derive(Serialize, Deserialize, Debug)]
pub struct McpToolInfoResponse {
    pub namespaced_name: String,
    pub original_name: String,
    pub description: Option<String>,
    pub server_name: String,
    pub annotations: Option<serde_json::Value>,
    pub pin_hash: Option<String>,
    pub approved: bool,
    pub pin_changed: bool,
}

/// Response for GET /mcp/policy.
#[derive(Serialize, Deserialize, Debug)]
pub struct McpPolicyInfoResponse {
    pub global_policy: Option<String>,
    pub default_tool_permission: String,
    pub blocked_servers: Vec<String>,
    pub tool_permissions: HashMap<String, String>,
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
fn default_history_limit() -> usize { 500 }
#[allow(dead_code)]
fn default_history_layer() -> String { "all".to_string() }

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
fn default_tail_lines() -> usize { 500 }

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
    fn provision_request_env_omitted() {
        let r = ProvisionRequest { name: None, ram_mb: Some(2048), cpus: Some(2), persistent: false, env: None, from: None };
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
        };
        let json = serde_json::to_string(&r).unwrap();
        let r2: ProvisionResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(r2.id, "vm-123");
        assert_eq!(r2.uds_path.as_deref(), Some(std::path::Path::new("/tmp/r/instances/vm-123.sock")));
    }

    // -----------------------------------------------------------------------
    // ListResponse
    // -----------------------------------------------------------------------

    #[test]
    fn list_response_empty() {
        let r = ListResponse { sandboxes: vec![], asset_health: None };
        let json = serde_json::to_string(&r).unwrap();
        let r2: ListResponse = serde_json::from_str(&json).unwrap();
        assert!(r2.sandboxes.is_empty());
    }

    #[test]
    fn list_response_multiple() {
        let r = ListResponse {
            sandboxes: vec![
                { let mut s = SandboxInfo::new("a".into(), 100, "Running".into(), true); s.name = Some("a".into()); s.ram_mb = Some(2048); s.cpus = Some(2); s },
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
        let r = PurgeResponse { purged: 5, persistent_purged: 2, ephemeral_purged: 3 };
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
        assert_eq!(r.timeout_secs, 60);
        assert_eq!(r.ram_mb, None);
        assert_eq!(r.cpus, None);
    }

    #[test]
    fn run_request_custom() {
        let json = json!({"command": "ls", "timeout_secs": 120, "ram_mb": 4096, "cpus": 4});
        let r: RunRequest = serde_json::from_value(json).unwrap();
        assert_eq!(r.timeout_secs, 120);
        assert_eq!(r.ram_mb, Some(4096));
        assert_eq!(r.cpus, Some(4));
    }

    // -----------------------------------------------------------------------
    // ExecRequest / ExecResponse
    // -----------------------------------------------------------------------

    #[test]
    fn exec_request_default_timeout() {
        let json = json!({"command": "echo hi"});
        let r: ExecRequest = serde_json::from_value(json).unwrap();
        assert_eq!(r.command, "echo hi");
        assert_eq!(r.timeout_secs, 30);
    }

    #[test]
    fn exec_request_custom_timeout() {
        let json = json!({"command": "sleep 10", "timeout_secs": 5});
        let r: ExecRequest = serde_json::from_value(json).unwrap();
        assert_eq!(r.timeout_secs, 5);
    }

    #[test]
    fn exec_response_roundtrip() {
        let r = ExecResponse { stdout: "hello\n".into(), stderr: "".into(), exit_code: 0 };
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
        let r = ReadFileResponse { content: "file contents".into() };
        let json = serde_json::to_string(&r).unwrap();
        let r2: ReadFileResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(r2.content, "file contents");
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
        let r = LogsResponse { logs: "Linux boot...\n".into(), serial_logs: None, process_logs: None };
        let json = serde_json::to_string(&r).unwrap();
        let r2: LogsResponse = serde_json::from_str(&json).unwrap();
        assert!(r2.logs.contains("Linux"));
    }

    #[test]
    fn error_response_roundtrip() {
        let r = ErrorResponse { error: "sandbox not found".into() };
        let json = serde_json::to_string(&r).unwrap();
        let r2: ErrorResponse = serde_json::from_str(&json).unwrap();
        assert!(r2.error.contains("not found"));
    }
}
