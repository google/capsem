use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Extensible MCP arguments and results retain their native JSON shape.
pub type Value = serde_json::Value;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum McpPermissionAction {
    Allow,
    Ask,
    Block,
    Preprocess,
    Rewrite,
    Postprocess,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
pub struct ProfileMcpInfoResponse {
    pub profile_id: String,
    pub server_count: usize,
    pub manual_server_count: usize,
    pub builtin_local_enabled: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
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

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
#[serde(transparent)]
pub struct McpServersListResponse(pub Vec<McpServerInfoResponse>);

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
pub struct McpDefaultPermissionResponse {
    pub action: McpPermissionAction,
    pub source: String,
    pub rule_id: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, ToSchema)]
pub struct McpToolInfoResponse {
    pub namespaced_name: String,
    pub original_name: String,
    pub description: Option<String>,
    pub server_name: String,
    /// MCP annotations are protocol-extensible JSON.
    pub annotations: Option<serde_json::Value>,
    pub pin_hash: Option<String>,
    pub pin_changed: bool,
    pub permission_action: McpPermissionAction,
    pub permission_source: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, ToSchema)]
#[serde(transparent)]
pub struct McpToolsListResponse(pub Vec<McpToolInfoResponse>);

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
pub struct McpRefreshResponse {
    pub success: bool,
    pub server_id: String,
    pub instances: usize,
}
