use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Namespace separator for MCP tool/prompt/resource names.
pub const NS_SEP: &str = "__";

/// A host-side MCP server definition (from user config or auto-detected).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerDef {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    /// Host-only credentials, never sent to the VM.
    pub env: HashMap<String, String>,
    pub enabled: bool,
    /// Where this definition came from: "claude", "gemini", "manual".
    pub source: String,
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
mod tests {
    use super::*;

    #[test]
    fn namespace_name_basic() {
        assert_eq!(namespace_name("github", "search_repos"), "github__search_repos");
    }

    #[test]
    fn parse_namespaced_basic() {
        let (server, original) = parse_namespaced("github__search_repos").unwrap();
        assert_eq!(server, "github");
        assert_eq!(original, "search_repos");
    }

    #[test]
    fn parse_namespaced_no_separator() {
        assert!(parse_namespaced("noseparator").is_none());
    }

    #[test]
    fn parse_namespaced_double_underscore_in_tool_name() {
        // Tool name itself contains __, split on FIRST only
        let (server, original) = parse_namespaced("github__my__tool").unwrap();
        assert_eq!(server, "github");
        assert_eq!(original, "my__tool");
    }

    #[test]
    fn namespace_roundtrip() {
        let ns = namespace_name("slack", "send_message");
        let (s, n) = parse_namespaced(&ns).unwrap();
        assert_eq!(s, "slack");
        assert_eq!(n, "send_message");
    }

    #[test]
    fn namespace_resource_uri_basic() {
        let uri = namespace_resource_uri("github", "repo://owner/repo");
        assert_eq!(uri, "capsem://github/repo://owner/repo");
    }

    #[test]
    fn parse_resource_uri_basic() {
        let (server, original) =
            parse_resource_uri("capsem://github/repo://owner/repo").unwrap();
        assert_eq!(server, "github");
        assert_eq!(original, "repo://owner/repo");
    }

    #[test]
    fn parse_resource_uri_nested_slashes() {
        let (server, original) =
            parse_resource_uri("capsem://fs/file:///home/user/doc.txt").unwrap();
        assert_eq!(server, "fs");
        assert_eq!(original, "file:///home/user/doc.txt");
    }

    #[test]
    fn parse_resource_uri_invalid() {
        assert!(parse_resource_uri("http://github/something").is_none());
        assert!(parse_resource_uri("capsem://").is_none());
    }

    #[test]
    fn resource_uri_roundtrip() {
        let uri = namespace_resource_uri("db", "postgres://localhost/mydb");
        let (s, u) = parse_resource_uri(&uri).unwrap();
        assert_eq!(s, "db");
        assert_eq!(u, "postgres://localhost/mydb");
    }

    #[test]
    fn json_rpc_request_serialize() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(serde_json::json!(1)),
            method: "tools/list".into(),
            params: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("tools/list"));
        let decoded: JsonRpcRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.method, "tools/list");
    }

    #[test]
    fn json_rpc_response_ok() {
        let resp = JsonRpcResponse::ok(Some(serde_json::json!(1)), serde_json::json!({"tools": []}));
        assert!(resp.error.is_none());
        assert!(resp.result.is_some());
    }

    #[test]
    fn json_rpc_response_err() {
        let resp = JsonRpcResponse::err(Some(serde_json::json!(1)), -32601, "method not found");
        assert!(resp.result.is_none());
        let err = resp.error.unwrap();
        assert_eq!(err.code, -32601);
        assert_eq!(err.message, "method not found");
    }

    #[test]
    fn json_rpc_notification_has_no_id() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: None,
            method: "notifications/initialized".into(),
            params: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("\"id\""));
    }
}
