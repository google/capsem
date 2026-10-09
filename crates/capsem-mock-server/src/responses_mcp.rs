//! Explicit MCP turns in the Responses API, through declared client tools.
use super::{
    agent_mcp::{proof_token, ECHO},
    now_unix,
};
use serde_json::{json, Value};

const CALL_ID: &str = "call_capsem_mcp_echo";

pub(super) fn reply(payload: &Value) -> Option<Result<Value, String>> {
    let token = proof_token(payload, "input")?;
    Some(token.and_then(|token| proof_reply(payload, token)))
}

fn proof_reply(payload: &Value, token: &str) -> Result<Value, String> {
    let additional = payload["input"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|item| item["type"] == "additional_tools" && item["role"] == "developer")
        .filter_map(|item| item["tools"].as_array())
        .flatten();
    let tools: Vec<_> = payload["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(additional)
        .collect();
    let direct = tools.iter().any(|tool| {
        tool["type"] == "function"
            && tool["name"] == ECHO
            && tool["parameters"]["properties"]["text"]["type"] == "string"
    });
    let code_mode = tools.iter().any(|tool| {
        tool["type"] == "namespace"
            && tool["name"] == "functions"
            && tool["tools"].as_array().is_some_and(|tools| {
                tools
                    .iter()
                    .any(|tool| tool["type"] == "custom" && tool["name"] == "exec")
            })
    });
    if !direct && !code_mode {
        return Err("MCP proof needs a declared echo tool or code executor".into());
    }
    let result = payload["input"].as_array().and_then(|input| {
        input.iter().rev().find(|item| {
            matches!(
                item["type"].as_str(),
                Some("function_call_output" | "custom_tool_call_output")
            )
        })
    });
    let item = if let Some(result) = result {
        if result["call_id"] != CALL_ID || !successful_echo(&result["output"], token, code_mode) {
            return Err("MCP proof needs the exact successful echo result for its call".into());
        }
        json!({"type": "message", "id": "msg_capsem_mcp_echo", "role": "assistant", "status": "completed",
            "content": [{"type": "output_text", "text": token, "annotations": [], "logprobs": []}]})
    } else if direct {
        json!({"type": "function_call", "id": "fc_capsem_mcp_echo", "call_id": CALL_ID,
            "name": ECHO, "arguments": json!({"text": token}).to_string(), "status": "completed"})
    } else {
        let code = format!("const entry = ALL_TOOLS.find(tool => tool.name.includes('capsem') && tool.name.endsWith('local__echo'));\n\
            if (!entry) throw new Error('Capsem echo unavailable');\n\
            const result = await tools[entry.name]({{text: '{token}'}});\n\
            if (result.isError) throw new Error('Capsem echo failed');\n\
            for (const block of result.content ?? []) if (block.type === 'text') text(block.text);");
        json!({"type": "custom_tool_call", "id": "ctc_capsem_mcp_echo", "call_id": CALL_ID,
            "namespace": "functions", "name": "exec", "input": code, "status": "completed"})
    };
    Ok(
        json!({"id": "resp_capsem_mcp_proof", "object": "response", "created_at": now_unix(),
        "status": "completed", "model": payload["model"].as_str().unwrap_or("mock-local"),
        "output": [item], "error": null, "incomplete_details": null,
        "usage": {"input_tokens": 31, "output_tokens": 17, "total_tokens": 48,
            "input_tokens_details": {"cached_tokens": 0}, "output_tokens_details": {"reasoning_tokens": 0}}}),
    )
}

fn successful_echo(output: &Value, token: &str, code_mode: bool) -> bool {
    if let Some(blocks) = output.as_array() {
        return code_mode
            && blocks.len() == 2
            && blocks.iter().all(|block| block["type"] == "input_text")
            && blocks[0]["text"]
                .as_str()
                .is_some_and(|header| header.starts_with("Script completed\n") && header.ends_with("\nOutput:\n"))
            && blocks[1]["text"] == token;
    }
    let mut text = output.as_str().unwrap_or("");
    if code_mode && text.starts_with("Script completed\n") {
        text = text.split_once("\nOutput:\n").map_or("", |(_, output)| output.trim());
    }
    if text == token {
        return true;
    }
    let parsed = serde_json::from_str::<Value>(text).ok();
    let value = parsed.as_ref().unwrap_or(output);
    (value["isError"].is_null() || value["isError"] == false)
        && value["content"] == json!([{"type": "text", "text": token}])
}
