//! Legacy Anthropic model turns used by hermetic clients.
use super::{agent_mcp, json_compact, json_response, response, shell_write_command, write_target, RespBody};
use bytes::Bytes;
use hyper::{Response, StatusCode};
use serde_json::{json, Value};

pub(super) fn reply(payload: Value) -> Response<RespBody> {
    let streaming = payload.get("stream").and_then(Value::as_bool) == Some(true);
    if let Some(proof) = agent_mcp::reply(&payload) {
        match proof {
            Err(error) => response(StatusCode::BAD_REQUEST, Bytes::from(error), "text/plain"),
            Ok(message) if streaming => response(StatusCode::OK, agent_mcp::stream(message), "text/event-stream"),
            Ok(message) => json_response(message),
        }
    } else if streaming {
        response(StatusCode::OK, anthropic_stream(payload), "text/event-stream")
    } else {
        json_response(anthropic_response(payload))
    }
}

fn anthropic_response(payload: Value) -> Value {
    let has_tool_result = serde_json::to_string(&payload)
        .map(|raw| raw.contains("\"type\":\"tool_result\""))
        .unwrap_or(false);
    let (token, path) = write_target(&payload, "claude");
    let model = payload
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("claude-sonnet-4-6");
    if has_tool_result {
        return json!({
            "id": "msg_capsem_mock_final",
            "type": "message",
            "role": "assistant",
            "model": model,
            "content": [
                {"type": "thinking", "thinking": "ledger reasoning"},
                {"type": "text", "text": token}
            ],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 7, "output_tokens": 17}
        });
    }

    let command = shell_write_command(&token, &path);
    json!({
        "id": "msg_capsem_mock",
        "type": "message",
        "role": "assistant",
        "model": model,
        "content": [
            {"type": "thinking", "thinking": "Plan file write"},
            {"type": "tool_use", "id": "toolu_capsem_write_poem", "name": "exec_command", "input": {"cmd": command}},
            {"type": "text", "text": token}
        ],
        "stop_reason": "tool_use",
        "usage": {"input_tokens": 33, "output_tokens": 27}
    })
}

fn anthropic_stream(payload: Value) -> Bytes {
    let has_tool_result = serde_json::to_string(&payload)
        .map(|raw| raw.contains("\"type\":\"tool_result\""))
        .unwrap_or(false);
    if has_tool_result {
        let (token, _) = write_target(&payload, "claude");
        let message = json!({
            "id": "msg_ironbank_final",
            "type": "message",
            "role": "assistant",
            "model": payload.get("model").and_then(Value::as_str).unwrap_or("claude-sonnet-4-6"),
            "content": [],
            "usage": {"input_tokens": 7, "output_tokens": 1}
        });
        return Bytes::from(format!(
            "event: message_start\ndata: {}\n\n\
event: content_block_start\ndata: {{\"type\":\"content_block_start\",\"index\":0,\"content_block\":{{\"type\":\"thinking\",\"thinking\":\"\"}}}}\n\n\
event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"thinking_delta\",\"thinking\":\"ledger reasoning\"}}}}\n\n\
event: content_block_stop\ndata: {{\"type\":\"content_block_stop\",\"index\":0}}\n\n\
event: content_block_start\ndata: {{\"type\":\"content_block_start\",\"index\":1,\"content_block\":{{\"type\":\"text\",\"text\":\"\"}}}}\n\n\
event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":1,\"delta\":{{\"type\":\"text_delta\",\"text\":\"{token}\"}}}}\n\n\
event: message_delta\ndata: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"end_turn\"}},\"usage\":{{\"output_tokens\":17}}}}\n\n\
event: message_stop\ndata: {{\"type\":\"message_stop\"}}\n\n",
            json_compact(json!({"type": "message_start", "message": message}))
        ));
    }
    if payload.get("tools").is_none() {
        let model = payload
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("claude-sonnet-4-6");
        return Bytes::from(format!(
            "event: message_start\ndata: {{\"type\":\"message_start\",\"message\":{{\"id\":\"msg_ironbank_stream_text\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"{model}\",\"content\":[],\"usage\":{{\"input_tokens\":25,\"output_tokens\":5}}}}}}\n\n\
event: content_block_start\ndata: {{\"type\":\"content_block_start\",\"index\":0,\"content_block\":{{\"type\":\"text\",\"text\":\"\"}}}}\n\n\
event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":\"Hello \"}}}}\n\n\
event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":\"world!\"}}}}\n\n\
event: content_block_stop\ndata: {{\"type\":\"content_block_stop\",\"index\":0}}\n\n\
event: message_delta\ndata: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"end_turn\"}},\"usage\":{{\"output_tokens\":5}}}}\n\n\
event: message_stop\ndata: {{\"type\":\"message_stop\"}}\n\n"
        ));
    }

    let (token, path) = write_target(&payload, "claude");
    let command = shell_write_command(&token, &path);
    let partial = json_compact(json!({
        "command": command,
        "description": "write ironbank token"
    }));
    let model = payload
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("claude-sonnet-4-6");
    Bytes::from(format!(
        "event: message_start\ndata: {{\"type\":\"message_start\",\"message\":{{\"id\":\"msg_ironbank_01\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"{model}\",\"content\":[],\"usage\":{{\"input_tokens\":31,\"output_tokens\":1}}}}}}\n\n\
event: content_block_start\ndata: {{\"type\":\"content_block_start\",\"index\":0,\"content_block\":{{\"type\":\"tool_use\",\"id\":\"toolu_capsem_write_poem\",\"name\":\"Bash\",\"input\":{{}}}}}}\n\n\
event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"input_json_delta\",\"partial_json\":{}}}}}\n\n\
event: content_block_stop\ndata: {{\"type\":\"content_block_stop\",\"index\":0}}\n\n\
event: message_delta\ndata: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"tool_use\",\"stop_sequence\":null}},\"usage\":{{\"output_tokens\":17}}}}\n\n\
event: message_stop\ndata: {{\"type\":\"message_stop\"}}\n\n",
        json_compact(json!(partial))
    ))
}
