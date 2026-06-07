use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// MCP server config (stored under [mcp])
// ---------------------------------------------------------------------------

/// MCP configuration from user.toml or corp.toml `[mcp]` sections.
///
/// This is server discovery/configuration only. MCP allow/ask/block decisions
/// are security rules over canonical MCP security events.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct McpUserConfig {
    /// Health check interval in seconds (default: 300).
    #[serde(default)]
    pub health_check_interval_secs: Option<u64>,
    /// Manually configured MCP servers.
    #[serde(default)]
    pub servers: Vec<McpManualServer>,
    /// Per-server enabled overrides (name -> enabled).
    #[serde(default)]
    pub server_enabled: HashMap<String, bool>,
}

/// A manually configured MCP server definition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpManualServer {
    pub name: String,
    /// HTTP endpoint URL for the MCP server.
    pub url: String,
    /// Custom HTTP headers to send with every request.
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Bearer token for Authorization header.
    #[serde(default)]
    pub bearer_token: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}
