//! Shared MCP configuration and wire identity.

use serde::{Deserialize, Serialize};

/// Authentication scheme for a remote MCP server.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum McpAuthKind {
    Bearer,
    OAuth,
}

/// Broker-owned authentication reference for a remote MCP server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct McpAuthConfig {
    pub kind: McpAuthKind,
    pub credential_ref: String,
}
