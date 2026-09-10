use crate::SandboxInfo;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone, ToSchema)]
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

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct ListResponse {
    pub sandboxes: Vec<SandboxInfo>,
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct ExecRequest {
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct ExecResponse {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    /// The guest produced more output than the per-exec cap allows, so
    /// `stdout` is a prefix. Defaulted so an older client still decodes.
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
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

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct PersistRequest {
    pub name: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct PurgeRequest {
    #[serde(default)]
    pub all: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct PurgeResponse {
    pub purged: u32,
    pub persistent_purged: u32,
    pub ephemeral_purged: u32,
}
