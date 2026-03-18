use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Namespace separator for MCP tool/prompt/resource names.
pub const NS_SEP: &str = "__";

/// A host-side MCP server definition (from user config or auto-detected).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerDef {
    pub name: String,
    /// HTTP endpoint URL for the MCP server.
    pub url: String,
    /// Custom HTTP headers to send with every request.
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Bearer token for Authorization header (extracted from env for convenience).
    #[serde(default)]
    pub bearer_token: Option<String>,
    pub enabled: bool,
    /// Where this definition came from: "claude", "gemini", "manual".
    pub source: String,
    /// True if this was auto-detected as a stdio/command server (display-only, not connectable).
    #[serde(default)]
    pub unsupported_stdio: bool,
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

    // ── ToolAnnotations tests ────────────────────────────────────────

    // ── to_mcp_json tests ─────────────────────────────────────────────

    #[test]
    fn to_mcp_json_uses_camel_case_keys() {
        let ann = ToolAnnotations {
            title: Some("Test Tool".into()),
            read_only_hint: true,
            destructive_hint: false,
            idempotent_hint: true,
            open_world_hint: false,
        };
        let json = ann.to_mcp_json();
        let obj = json.as_object().unwrap();
        // Must have camelCase keys
        assert!(obj.contains_key("readOnlyHint"));
        assert!(obj.contains_key("destructiveHint"));
        assert!(obj.contains_key("idempotentHint"));
        assert!(obj.contains_key("openWorldHint"));
        assert!(obj.contains_key("title"));
        // Must NOT have snake_case keys
        assert!(!obj.contains_key("read_only_hint"));
        assert!(!obj.contains_key("destructive_hint"));
        assert!(!obj.contains_key("idempotent_hint"));
        assert!(!obj.contains_key("open_world_hint"));
        // Values correct
        assert_eq!(obj["readOnlyHint"], true);
        assert_eq!(obj["destructiveHint"], false);
        assert_eq!(obj["idempotentHint"], true);
        assert_eq!(obj["openWorldHint"], false);
        assert_eq!(obj["title"], "Test Tool");
    }

    #[test]
    fn to_mcp_json_omits_title_when_none() {
        let ann = ToolAnnotations::default();
        let json = ann.to_mcp_json();
        let obj = json.as_object().unwrap();
        assert!(!obj.contains_key("title"));
        assert_eq!(obj.len(), 4); // only the 4 bool hints
    }

    #[test]
    fn to_mcp_json_default_annotations_correct() {
        let ann = ToolAnnotations::default();
        let json = ann.to_mcp_json();
        let obj = json.as_object().unwrap();
        assert_eq!(obj["readOnlyHint"], false);
        assert_eq!(obj["destructiveHint"], true);
        assert_eq!(obj["idempotentHint"], false);
        assert_eq!(obj["openWorldHint"], true);
    }

    #[test]
    fn tool_annotations_defaults() {
        let ann = ToolAnnotations::default();
        assert!(!ann.read_only_hint);
        assert!(ann.destructive_hint); // default true
        assert!(!ann.idempotent_hint);
        assert!(ann.open_world_hint); // default true
        assert!(ann.title.is_none());
    }

    #[test]
    fn tool_annotations_from_json() {
        let json = serde_json::json!({
            "title": "Read file",
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": false
        });
        let ann: ToolAnnotations = serde_json::from_value(json).unwrap();
        assert_eq!(ann.title.as_deref(), Some("Read file"));
        assert!(ann.read_only_hint);
        assert!(!ann.destructive_hint);
        assert!(ann.idempotent_hint);
        assert!(!ann.open_world_hint);
    }

    #[test]
    fn tool_annotations_snake_case_also_works() {
        let json = serde_json::json!({
            "read_only_hint": true,
            "destructive_hint": false
        });
        let ann: ToolAnnotations = serde_json::from_value(json).unwrap();
        assert!(ann.read_only_hint);
        assert!(!ann.destructive_hint);
    }

    #[test]
    fn tool_annotations_missing_field_uses_defaults() {
        let json = serde_json::json!({});
        let ann: ToolAnnotations = serde_json::from_value(json).unwrap();
        assert!(!ann.read_only_hint);
        assert!(ann.destructive_hint);
    }

    #[test]
    fn tool_annotations_extra_fields_ignored() {
        let json = serde_json::json!({
            "readOnlyHint": true,
            "unknownField": "whatever",
            "customAnnotation": 42
        });
        // Should not fail on unknown fields
        let ann: ToolAnnotations = serde_json::from_value(json).unwrap();
        assert!(ann.read_only_hint);
    }

    #[test]
    fn tool_def_with_annotations() {
        let def = McpToolDef {
            namespaced_name: "github__search".into(),
            original_name: "search".into(),
            description: Some("Search repos".into()),
            input_schema: serde_json::json!({}),
            server_name: "github".into(),
            annotations: Some(ToolAnnotations {
                read_only_hint: true,
                ..Default::default()
            }),
        };
        assert!(def.annotations.unwrap().read_only_hint);
    }

    #[test]
    fn tool_def_without_annotations() {
        let def = McpToolDef {
            namespaced_name: "test__tool".into(),
            original_name: "tool".into(),
            description: None,
            input_schema: serde_json::json!({}),
            server_name: "test".into(),
            annotations: None,
        };
        assert!(def.annotations.is_none());
    }
}
