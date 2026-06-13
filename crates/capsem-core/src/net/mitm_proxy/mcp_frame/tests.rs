use serde_json::json;

use super::*;

fn request(method: &str, params: serde_json::Value) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(json!(1)),
        method: method.to_string(),
        params: Some(params),
        meta: None,
    }
}

#[test]
fn log_attribution_reads_tool_namespace() {
    let req = request("tools/call", json!({"name": "local__echo"}));

    let (server_name, tool_name) = mcp_log_attribution(&req);

    assert_eq!(server_name, "local");
    assert_eq!(tool_name.as_deref(), Some("local__echo"));
}

#[test]
fn log_attribution_reads_resource_namespace() {
    let req = request(
        "resources/read",
        json!({"uri": "capsem://slowlist/doc://slow"}),
    );

    let (server_name, tool_name) = mcp_log_attribution(&req);

    assert_eq!(server_name, "slowlist");
    assert!(tool_name.is_none());
}

#[test]
fn log_attribution_reads_prompt_namespace() {
    let req = request("prompts/get", json!({"name": "writer__poem"}));

    let (server_name, tool_name) = mcp_log_attribution(&req);

    assert_eq!(server_name, "writer");
    assert!(tool_name.is_none());
}
