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
fn parse_namespaced_ambiguous_server_name() {
    // If a server name contains __, it breaks namespacing for its tools
    // if split-on-first is used. This confirms we must either forbid __
    // in server names or use split-on-last.
    let (server, original) = parse_namespaced("a__b__c").unwrap();
    assert_eq!(server, "a");
    assert_eq!(original, "b__c");
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
    let (server, original) = parse_resource_uri("capsem://github/repo://owner/repo").unwrap();
    assert_eq!(server, "github");
    assert_eq!(original, "repo://owner/repo");
}

#[test]
fn parse_resource_uri_nested_slashes() {
    let (server, original) = parse_resource_uri("capsem://fs/file:///home/user/doc.txt").unwrap();
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
        meta: None,
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
        meta: None,
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
        timeout_secs: None,
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
        timeout_secs: None,
    };
    assert!(def.annotations.is_none());
}
