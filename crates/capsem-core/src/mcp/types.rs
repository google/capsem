use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Namespace separator for MCP tool/prompt/resource names.
pub const NS_SEP: &str = "__";

/// Auth material for remote MCP servers.
///
/// The TOML contract stores only brokered credential references. Raw API keys,
/// OAuth access tokens, refresh tokens, or Authorization headers must stay
/// inside the credential broker.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum McpAuthKind {
    Bearer,
    OAuth,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct McpAuthConfig {
    pub kind: McpAuthKind,
    pub credential_ref: String,
}

/// A host-side MCP server definition (from user config or auto-detected).
///
/// Transport is determined by which fields are set:
/// - `command` is Some => stdio transport (spawn subprocess)
/// - `url` is non-empty => HTTP transport
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerDef {
    pub name: String,
    /// HTTP endpoint URL for the MCP server (empty for stdio servers).
    #[serde(default)]
    pub url: String,
    /// Binary path for stdio-transport servers (None for HTTP servers).
    #[serde(default)]
    pub command: Option<String>,
    /// Command-line arguments for stdio-transport servers.
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables to pass to stdio-transport servers.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Custom HTTP headers to send with every request.
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Broker-owned auth material for remote MCP servers.
    #[serde(default)]
    pub auth: Option<McpAuthConfig>,
    pub enabled: bool,
    /// Where this definition came from: "claude", "gemini", "manual", "builtin".
    pub source: String,
    /// Number of independent stdio subprocess peers to spawn for this
    /// server. `None` or `Some(0|1)` ⇒ single peer (default behavior, no
    /// pool). HTTP servers ignore this field — HTTP/2 already multiplexes,
    /// so pooling buys nothing at the transport level.
    ///
    /// Used to remove rmcp's per-Peer stdio driver as a singleton funnel:
    /// each `RunningService<RoleClient, ()>` owns one mpsc → one driver
    /// task → one stdin pipe. N peers = N independent funnels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pool_size: Option<u32>,
    /// Original tool names (post-namespace strip) that are safe to
    /// round-robin across pool peers. Tools NOT in this list pin to
    /// `peers[0]` so per-process state (e.g. the builtin's
    /// `Arc<Mutex<AutoSnapshotScheduler>>`) stays consistent. Empty
    /// list ⇒ all tools pin to peers[0] (no fan-out).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pool_safe_tools: Vec<String>,
}

impl McpServerDef {
    /// True if this server uses stdio transport (subprocess).
    pub fn is_stdio(&self) -> bool {
        self.command.is_some()
    }
}

/// MCP tool annotations (per MCP spec 2024-11-05).
///
/// Displayed as informational hints in the UI. Per MCP spec:
/// "Clients MUST NOT rely solely on these for security decisions."
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolAnnotations {
    /// Human-readable title for the tool.
    #[serde(default)]
    pub title: Option<String>,
    /// Whether the tool only reads data (no side effects).
    #[serde(default, alias = "readOnlyHint")]
    pub read_only_hint: bool,
    /// Whether the tool may perform destructive operations.
    #[serde(default = "default_true", alias = "destructiveHint")]
    pub destructive_hint: bool,
    /// Whether calling the tool multiple times with same args has same effect.
    #[serde(default, alias = "idempotentHint")]
    pub idempotent_hint: bool,
    /// Whether the tool may interact with external entities.
    #[serde(default = "default_true", alias = "openWorldHint")]
    pub open_world_hint: bool,
}

impl ToolAnnotations {
    /// Serialize to MCP wire format (camelCase keys per MCP spec 2024-11-05).
    ///
    /// The struct uses snake_case for Tauri IPC (frontend), but the JSON-RPC
    /// wire protocol requires camelCase. This method produces the correct
    /// wire representation.
    pub fn to_mcp_json(&self) -> serde_json::Value {
        let mut obj = serde_json::Map::new();
        if let Some(ref title) = self.title {
            obj.insert("title".into(), serde_json::Value::String(title.clone()));
        }
        obj.insert("readOnlyHint".into(), self.read_only_hint.into());
        obj.insert("destructiveHint".into(), self.destructive_hint.into());
        obj.insert("idempotentHint".into(), self.idempotent_hint.into());
        obj.insert("openWorldHint".into(), self.open_world_hint.into());
        serde_json::Value::Object(obj)
    }
}

impl Default for ToolAnnotations {
    fn default() -> Self {
        Self {
            title: None,
            read_only_hint: false,
            destructive_hint: true,
            idempotent_hint: false,
            open_world_hint: true,
        }
    }
}

fn default_true() -> bool {
    true
}

/// A tool discovered from a server's tools/list response, with namespaced name.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolDef {
    /// Namespaced name exposed to the agent (e.g. "github__search_repos").
    pub namespaced_name: String,
    /// Original name sent to the real server (e.g. "search_repos").
    pub original_name: String,
    pub description: Option<String>,
    pub input_schema: serde_json::Value,
    pub server_name: String,
    /// MCP tool annotations (untrusted hints from the server).
    #[serde(default)]
    pub annotations: Option<ToolAnnotations>,
    /// Optional host-side execution timeout hint from the aggregator catalog.
    /// This is not exposed on the MCP wire; the MITM endpoint clamps it to
    /// its configured tool-call ceiling before use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
}

/// A resource discovered from a server's resources/list response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpResourceDef {
    /// Namespaced URI (e.g. "capsem://github/repo://owner/repo").
    pub namespaced_uri: String,
    /// Original URI (e.g. "repo://owner/repo").
    pub original_uri: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub mime_type: Option<String>,
    pub server_name: String,
}

/// A prompt discovered from a server's prompts/list response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpPromptDef {
    /// Namespaced name (e.g. "github__review_pr").
    pub namespaced_name: String,
    /// Original name (e.g. "review_pr").
    pub original_name: String,
    pub description: Option<String>,
    pub arguments: Vec<serde_json::Value>,
    pub server_name: String,
}

// ── JSON-RPC 2.0 types ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<serde_json::Value>,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
    /// W5: optional W3C trace context propagated in band so a
    /// per-tool-call trace can be carried even when the underlying
    /// stdio transport doesn't support headers.
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<JsonRpcMeta>,
}

/// W3C trace context envelope on JSON-RPC `_meta`. All fields optional.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JsonRpcMeta {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub traceparent: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tracestate: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
    /// W5: echo back so the caller can cross-check the endpoint's trace.
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<JsonRpcMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl JsonRpcResponse {
    /// Create a successful response.
    pub fn ok(id: Option<serde_json::Value>, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(result),
            error: None,
            meta: None,
        }
    }

    /// Create an error response.
    pub fn err(id: Option<serde_json::Value>, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data: None,
            }),
            meta: None,
        }
    }
}

// ── Namespace helpers ────────────────────────────────────────────────

/// Create a namespaced name: "github" + "search_repos" -> "github__search_repos"
pub fn namespace_name(server: &str, name: &str) -> String {
    format!("{server}{NS_SEP}{name}")
}

/// Parse a namespaced name back to (server, original). Splits on first `__` only.
/// Returns None if no separator found.
pub fn parse_namespaced(namespaced: &str) -> Option<(&str, &str)> {
    namespaced.find(NS_SEP).map(|pos| {
        let server = &namespaced[..pos];
        let original = &namespaced[pos + NS_SEP.len()..];
        (server, original)
    })
}

/// Create a namespaced resource URI: "capsem://github/repo://owner/repo"
pub fn namespace_resource_uri(server: &str, uri: &str) -> String {
    format!("capsem://{server}/{uri}")
}

/// Parse a namespaced resource URI back to (server, original_uri).
/// Input: "capsem://github/repo://owner/repo" -> ("github", "repo://owner/repo")
pub fn parse_resource_uri(namespaced: &str) -> Option<(&str, &str)> {
    let rest = namespaced.strip_prefix("capsem://")?;
    let slash_pos = rest.find('/')?;
    let server = &rest[..slash_pos];
    let original_uri = &rest[slash_pos + 1..];
    Some((server, original_uri))
}

#[cfg(test)]
mod tests;
