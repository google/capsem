pub use capsem_api::*;
use capsem_core::net::policy_config::SecurityRuleAction;
use serde::{Deserialize, Serialize};
use std::net::Ipv4Addr;

/// Internal owner-authenticated private-name lookup.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct PrivateResolveRequest {
    pub source_vm: String,
    pub owner_secret: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<Ipv4Addr>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct PrivateResolveResponse {
    pub name: String,
    pub address: Ipv4Addr,
    pub vm: String,
    pub network: String,
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
    pub assets: AssetStatus,
    pub corp: serde_json::Value,
    pub updates: UpdateStatusResponse,
}

pub fn mcp_permission_action(action: SecurityRuleAction) -> McpPermissionAction {
    match action {
        SecurityRuleAction::Allow => McpPermissionAction::Allow,
        SecurityRuleAction::Ask => McpPermissionAction::Ask,
        SecurityRuleAction::Block => McpPermissionAction::Block,
        SecurityRuleAction::Preprocess => McpPermissionAction::Preprocess,
        SecurityRuleAction::Rewrite => McpPermissionAction::Rewrite,
        SecurityRuleAction::Postprocess => McpPermissionAction::Postprocess,
    }
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
