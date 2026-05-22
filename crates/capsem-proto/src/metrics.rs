use serde::{Deserialize, Serialize};

pub const METRICS_SCHEMA_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct VmMetricsSnapshot {
    pub schema_version: u32,
    pub vm_id: String,
    pub persistent: bool,
    pub lifecycle: VmLifecycleMetrics,
    pub resources: VmResourceMetrics,
    pub ask: VmAskMetrics,
    pub http: VmHttpMetrics,
    pub dns: VmDnsMetrics,
    pub model: VmModelMetrics,
    pub mcp: VmMcpMetrics,
    pub filesystem: VmFilesystemMetrics,
    pub security: VmSecurityMetrics,
    pub captured_at_unix_ms: u64,
}

impl VmMetricsSnapshot {
    pub fn empty(vm_id: impl Into<String>, persistent: bool, captured_at_unix_ms: u64) -> Self {
        Self {
            schema_version: METRICS_SCHEMA_VERSION,
            vm_id: vm_id.into(),
            persistent,
            lifecycle: VmLifecycleMetrics::default(),
            resources: VmResourceMetrics::default(),
            ask: VmAskMetrics::default(),
            http: VmHttpMetrics::default(),
            dns: VmDnsMetrics::default(),
            model: VmModelMetrics::default(),
            mcp: VmMcpMetrics::default(),
            filesystem: VmFilesystemMetrics::default(),
            security: VmSecurityMetrics::default(),
            captured_at_unix_ms,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct VmLifecycleMetrics {
    pub state: String,
    pub uptime_secs: u64,
    pub boot_count: u64,
    pub restart_count: u64,
    pub suspend_count: u64,
    pub resume_count: u64,
    pub shutdown_count: u64,
    pub unexpected_exit_count: u64,
    pub last_transition_unix_ms: Option<u64>,
    pub last_error: Option<String>,
}

impl Default for VmLifecycleMetrics {
    fn default() -> Self {
        Self {
            state: "unknown".to_string(),
            uptime_secs: 0,
            boot_count: 0,
            restart_count: 0,
            suspend_count: 0,
            resume_count: 0,
            shutdown_count: 0,
            unexpected_exit_count: 0,
            last_transition_unix_ms: None,
            last_error: None,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct VmResourceMetrics {
    pub configured_ram_mb: u64,
    pub configured_vcpus: u32,
    pub host_pid: Option<u32>,
    pub host_process_rss_bytes: Option<u64>,
    pub host_cpu_time_micros: Option<u64>,
    pub host_cpu_percent: Option<f64>,
    pub session_disk_bytes: Option<u64>,
    pub workspace_disk_bytes: Option<u64>,
    pub rootfs_overlay_bytes: Option<u64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct VmAskMetrics {
    pub total_asks: u64,
    pub asks_allowed: u64,
    pub asks_denied: u64,
    pub asks_errored: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct VmHttpMetrics {
    pub http_requests_total: u64,
    pub http_requests_allowed_total: u64,
    pub http_requests_warned_total: u64,
    pub http_requests_denied_total: u64,
    pub http_requests_errored_total: u64,
    pub http_bytes_sent_total: u64,
    pub http_bytes_received_total: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct VmDnsMetrics {
    pub dns_queries_total: u64,
    pub dns_queries_allowed_total: u64,
    pub dns_queries_warned_total: u64,
    pub dns_queries_denied_total: u64,
    pub dns_queries_rewritten_total: u64,
    pub dns_queries_errored_total: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct VmModelMetrics {
    pub model_requests_total: u64,
    pub model_requests_allowed_total: u64,
    pub model_requests_warned_total: u64,
    pub model_requests_denied_total: u64,
    pub model_requests_errored_total: u64,
    pub model_input_tokens_total: u64,
    pub model_output_tokens_total: u64,
    pub model_estimated_cost_micros_total: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct VmMcpMetrics {
    pub mcp_tool_invocations_total: u64,
    pub mcp_tool_invocations_allowed_total: u64,
    pub mcp_tool_invocations_warned_total: u64,
    pub mcp_tool_invocations_denied_total: u64,
    pub mcp_tool_invocations_errored_total: u64,
    pub mcp_servers_connected_total: u64,
    pub mcp_servers_disconnected_total: u64,
    pub mcp_server_errors_total: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct VmFilesystemMetrics {
    pub fs_reads_total: u64,
    pub fs_writes_total: u64,
    pub fs_creates_total: u64,
    pub fs_deletes_total: u64,
    pub fs_restores_total: u64,
    pub fs_errors_total: u64,
    pub fs_bytes_read_total: u64,
    pub fs_bytes_written_total: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct VmSecurityMetrics {
    pub security_events_total: u64,
    pub enforcement_decisions_total: u64,
    pub detection_findings_total: u64,
    pub blocks_total: u64,
    pub asks_total: u64,
    pub rewrites_total: u64,
    pub throttles_total: u64,
    pub errors_total: u64,
    pub latest_block_event_id: Option<String>,
    pub latest_block_rule_id: Option<String>,
    pub latest_block_reason: Option<String>,
    pub latest_block_unix_ms: Option<u64>,
    pub latest_detection_event_id: Option<String>,
    pub latest_detection_rule_id: Option<String>,
    pub latest_detection_title: Option<String>,
    pub latest_detection_severity: Option<String>,
    pub latest_detection_unix_ms: Option<u64>,
}
