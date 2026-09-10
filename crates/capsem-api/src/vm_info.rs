use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Recorded model and tool activity for this VM's session.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct VmAiInfo {
    pub model_call_count: u64,
    pub total_input_tokens: u64,
    pub total_thinking_tokens: u64,
    pub total_output_tokens: u64,
    pub total_tool_calls: u64,
    pub total_estimated_cost_usd: f64,
    /// Models observed in the session, grouped by provider and model identity.
    pub models: Vec<ModelUsage>,
    /// Recorded MCP tool calls, grouped by server and tool identity.
    pub mcp: Vec<McpUsage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ModelUsage {
    /// Provider identifiers and model names are open namespaces.
    pub provider: String,
    pub model: String,
    pub call_count: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub estimated_cost_usd: f64,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct McpUsage {
    pub server_name: Option<String>,
    pub tool_name: String,
    pub call_count: u64,
    pub duration_ms: u64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct VmNetworkInfo {
    pub total_requests: u64,
    pub allowed_requests: u64,
    pub denied_requests: u64,
    pub errors: u64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FileEventAction {
    Created,
    Modified,
    Deleted,
    Restored,
    Read,
    Imported,
    Exported,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FileActionCount {
    pub action: FileEventAction,
    pub count: u64,
}

/// Recorded filesystem activity, not a recursive workspace inventory.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct VmFilesInfo {
    pub total_events: u64,
    pub actions: Vec<FileActionCount>,
}
