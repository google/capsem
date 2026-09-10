pub use capsem_api::*;
use capsem_core::net::policy_config::{DetectionLevel, ProfileConfigFile, SecurityRuleAction};
use capsem_core::session::{GlobalStats, McpToolSummary, ProviderSummary, SessionRecord, ToolSummary};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Response for GET /stats -- global session stats from the logger DB boundary.
#[derive(Serialize, Debug, Clone)]
pub struct StatsResponse {
    pub global: GlobalStats,
    pub sessions: Vec<SessionRecord>,
    pub top_providers: Vec<ProviderSummary>,
    pub top_tools: Vec<ToolSummary>,
    pub top_mcp_tools: Vec<McpToolSummary>,
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
