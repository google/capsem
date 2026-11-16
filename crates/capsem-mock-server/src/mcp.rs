//! Shared deterministic MCP upstream, including bounded hostile-response fixtures.
use serde_json::{json, Value};

const MAX_FIXTURE_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

pub(crate) fn response(payload: Value) -> Value {
    let id = payload.get("id").cloned().unwrap_or(json!(1));
    let method = payload.get("method").and_then(Value::as_str).unwrap_or("");
    match method {
        "initialize" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {"listChanged": false}, "resources": {}},
                "serverInfo": {"name": "capsem-mock-server", "version": "1.0.0"}
            }
        }),
        "tools/list" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "tools": [
                    {"name": "fixture_lookup", "description": "Return deterministic debug content.", "inputSchema": {"type": "object", "properties": {"query": {"type": "string"}, "response_bytes": {"type": "integer", "minimum": 0, "maximum": MAX_FIXTURE_RESPONSE_BYTES, "description": "Optional exact text response size for transport acceptance."}}}},
                    {"name": "fetch_http", "description": "Fetch a local mock server URL.", "inputSchema": {"type": "object", "properties": {"url": {"type": "string"}}}},
                    {"name": "slow_sleep", "description": "Sleep before returning deterministic text.", "inputSchema": {"type": "object", "properties": {}}}
                ]
            }
        }),
        "tools/call" => {
            let name = payload
                .pointer("/params/name")
                .and_then(Value::as_str)
                .unwrap_or("fixture_lookup");
            let mut text = format!("capsem-mock-server:mcp:{name}");
            let mut is_error = false;
            if name == "fixture_lookup" {
                if let Some(value) = payload.pointer("/params/arguments/response_bytes") {
                    match value
                        .as_u64()
                        .and_then(|value| usize::try_from(value).ok())
                        .filter(|value| *value <= MAX_FIXTURE_RESPONSE_BYTES)
                    {
                        Some(bytes) => {
                            // This fixture's prefix is ASCII. Generate directly
                            // into the final bounded text, without a padding copy.
                            text.truncate(bytes);
                            text.extend(std::iter::repeat_n('x', bytes.saturating_sub(text.len())));
                        }
                        None => {
                            text = format!("response_bytes must be an integer from 0 to {MAX_FIXTURE_RESPONSE_BYTES}");
                            is_error = true;
                        }
                    }
                }
            }
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "content": [{"type": "text", "text": text}],
                    "isError": is_error
                }
            })
        }
        "resources/list" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "resources": [{
                    "uri": "doc://slow",
                    "name": "slow-doc",
                    "description": "Slow deterministic resource.",
                    "mimeType": "text/plain"
                }]
            }
        }),
        "resources/read" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "contents": [{"uri": payload.pointer("/params/uri").and_then(Value::as_str).unwrap_or("doc://unknown"), "mimeType": "text/plain", "text": "capsem-mock-server:mcp:resource"}]
            }
        }),
        _ => {
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "method not found"}})
        }
    }
}

pub(crate) fn should_delay(payload: &Value) -> bool {
    match payload.get("method").and_then(Value::as_str) {
        Some("tools/call") => payload.pointer("/params/name").and_then(Value::as_str) == Some("slow_sleep"),
        Some("resources/read") => payload
            .pointer("/params/uri")
            .and_then(Value::as_str)
            .is_some_and(|uri| uri == "doc://slow" || uri.ends_with("/doc://slow")),
        _ => false,
    }
}

#[cfg(test)]
mod tests;
