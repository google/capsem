//! Responses API fixture dispatch and ordinary model turns.
use super::{json_response, now_unix, response, responses_mcp, shell_write_command, write_target, RespBody};
mod stream;

use bytes::Bytes;
use hyper::{Response, StatusCode};
use serde_json::{json, Value};

pub(super) fn reply(payload: Value) -> Response<RespBody> {
    let streaming = payload.get("stream").and_then(Value::as_bool) == Some(true);
    let message = match message(&payload) {
        Ok(message) => message,
        Err(error) => return response(StatusCode::BAD_REQUEST, Bytes::from(error), "text/plain"),
    };
    if streaming {
        response(StatusCode::OK, stream::stream(message), "text/event-stream")
    } else {
        json_response(message)
    }
}

pub(super) fn message(payload: &Value) -> Result<Value, String> {
    responses_mcp::reply(payload).unwrap_or_else(|| Ok(ordinary(payload)))
}

pub(super) fn events(message: Value) -> Vec<Value> {
    stream::events(message)
}

fn ordinary(payload: &Value) -> Value {
    let (token, path) = write_target(payload, "codex-cli");
    let final_turn = payload["input"]
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item["type"] == "function_call_output"));
    let item = if final_turn {
        json!({"id": "msg_capsem_mock", "type": "message", "status": "completed", "role": "assistant",
            "content": [{"type": "output_text", "text": token, "annotations": [], "logprobs": []}]})
    } else {
        json!({"id": "fc_capsem_mock", "type": "function_call", "status": "completed",
            "call_id": format!("call_{}", &token[..token.len().min(12)]), "name": "exec_command",
            "arguments": json!({"cmd": shell_write_command(&token, &path), "yield_time_ms": 1000, "max_output_tokens": 2000}).to_string()})
    };
    let (input, output) = if final_turn { (7, 5) } else { (31, 17) };
    json!({"id": "resp_capsem_mock", "object": "response", "created_at": now_unix(), "status": "completed",
        "model": payload["model"].as_str().unwrap_or("mock-local"),
        "output": [item, {"id": "rs_capsem_mock", "type": "reasoning", "status": "completed",
            "summary": [{"type": "summary_text", "text": "ledger reasoning"}]}],
        "error": null, "incomplete_details": null,
        "usage": {"input_tokens": input, "output_tokens": output, "total_tokens": input + output,
            "input_tokens_details": {"cached_tokens": 0}, "output_tokens_details": {"reasoning_tokens": 2}}})
}
