//! An explicit MCP proof turn asks for a declared tool and consumes its result.
use bytes::Bytes;
use serde_json::{json, Value};

const MARKER: &str = "CAPSEM_MCP_PROOF=";
const CALL_ID: &str = "toolu_capsem_mcp_echo";
const DISCOVERY_ID: &str = "toolu_capsem_mcp_discover";
const ECHO: &str = "mcp__capsem__local__echo";

pub(super) fn reply(payload: &Value) -> Option<Result<Value, String>> {
    payload.get("tools")?;
    let messages = payload.get("messages")?.as_array()?;
    let prompt = messages
        .iter()
        .filter(|message| message["role"] == "user")
        .find_map(|message| {
            let content = &message["content"];
            content.as_str().filter(|text| text.contains(MARKER)).or_else(|| {
                content
                    .as_array()?
                    .iter()
                    .filter(|block| block["type"] == "text")
                    .find_map(|block| block["text"].as_str().filter(|text| text.contains(MARKER)))
            })
        })?;
    Some(proof_reply(payload, messages, prompt))
}

fn proof_reply(payload: &Value, messages: &[Value], prompt: &str) -> Result<Value, String> {
    let token = prompt.split_once(MARKER).unwrap().1.split('.').next().unwrap();
    if token.len() != 32 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("MCP proof needs a UUID hex token".into());
    }
    let tools = payload["tools"].as_array().ok_or("MCP proof needs declared tools")?;
    let results: Vec<_> = messages
        .iter()
        .filter(|message| message["role"] == "user")
        .filter_map(|message| message["content"].as_array())
        .flatten()
        .filter(|block| block["type"] == "tool_result")
        .collect();
    let latest = results.last().copied();
    let discovery = latest.filter(|result| result["tool_use_id"] == DISCOVERY_ID);
    let declared = tools.iter().any(|tool| tool["name"] == ECHO) || discovery.is_some_and(discovered_echo);
    if !declared {
        let available = messages
            .iter()
            .any(|message| message["role"] == "system" && message["content"].to_string().contains(ECHO));
        if results.is_empty() && available && tools.iter().any(|tool| tool["name"] == "ToolSearch") {
            return Ok(message(
                payload,
                json!([{"type": "tool_use", "id": DISCOVERY_ID,
                "name": "ToolSearch", "input": {"query": format!("select:{ECHO}"), "max_results": 1}}]),
                "tool_use",
            ));
        }
        return Err("MCP proof needs the declared Capsem echo tool".into());
    }
    if discovery.is_some_and(|result| !discovered_echo(result)) {
        return Err("MCP proof needs a successful echo schema discovery".into());
    }
    let (content, stop) = if results.is_empty() || discovery.is_some() {
        (
            json!([{"type": "tool_use", "id": CALL_ID, "name": ECHO, "input": {"text": token}}]),
            "tool_use",
        )
    } else {
        let result = results.last().unwrap();
        let echoed =
            result["content"].as_str() == Some(token) || result["content"] == json!([{"type": "text", "text": token}]);
        if result["tool_use_id"] != CALL_ID || result["is_error"] == true || !echoed {
            return Err("MCP proof needs the exact successful echo result for its call".into());
        }
        (json!([{"type": "text", "text": token}]), "end_turn")
    };
    Ok(message(payload, content, stop))
}

fn discovered_echo(result: &Value) -> bool {
    if result["is_error"] == true {
        return false;
    }
    let Some(blocks) = result["content"].as_array() else {
        return false;
    };
    blocks.iter().filter_map(|block| block["text"].as_str()).any(|text| {
        text.split("<function>")
            .skip(1)
            .filter_map(|tail| tail.split_once("</function>"))
            .filter_map(|(schema, _)| serde_json::from_str::<Value>(schema).ok())
            .any(|schema| schema["name"] == ECHO && schema["parameters"]["properties"]["text"]["type"] == "string")
    })
}

fn message(payload: &Value, content: Value, stop: &str) -> Value {
    json!({"id": "msg_capsem_mcp_proof", "type": "message", "role": "assistant",
        "model": payload["model"].as_str().unwrap_or("claude-sonnet-4-6"),
        "content": content, "stop_reason": stop, "stop_sequence": null,
        "usage": {"input_tokens": 31, "output_tokens": 17}})
}

pub(super) fn stream(mut message: Value) -> Bytes {
    let blocks = message["content"].take();
    message["content"] = json!([]);
    let stop = message["stop_reason"].take();
    message["stop_sequence"] = Value::Null;
    let mut output = String::new();
    let mut event = |name: &str, data: Value| {
        output.push_str(&format!("event: {name}\ndata: {data}\n\n"));
    };
    event("message_start", json!({"type": "message_start", "message": message}));
    for (index, mut block) in blocks.as_array().unwrap().iter().cloned().enumerate() {
        let delta = if block["type"] == "tool_use" {
            let input = block["input"].take().to_string();
            block["input"] = json!({});
            json!({"type": "input_json_delta", "partial_json": input})
        } else {
            let text = block["text"].take();
            block["text"] = json!("");
            json!({"type": "text_delta", "text": text})
        };
        event(
            "content_block_start",
            json!({"type": "content_block_start", "index": index, "content_block": block}),
        );
        event(
            "content_block_delta",
            json!({"type": "content_block_delta", "index": index, "delta": delta}),
        );
        event(
            "content_block_stop",
            json!({"type": "content_block_stop", "index": index}),
        );
    }
    event(
        "message_delta",
        json!({"type": "message_delta", "delta": {"stop_reason": stop, "stop_sequence": null}, "usage": {"output_tokens": 17}}),
    );
    event("message_stop", json!({"type": "message_stop"}));
    Bytes::from(output)
}
