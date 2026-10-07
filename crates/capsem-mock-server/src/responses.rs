//! Responses API fixture dispatch and legacy model turns.
use super::{
    json_compact, json_response, response, responses_mcp, shell_write_command, write_target, RespBody, EXPECTED_POEM,
};
use bytes::Bytes;
use hyper::{Response, StatusCode};
use serde_json::{json, Value};

pub(super) fn reply(payload: Value) -> Response<RespBody> {
    let streaming = payload.get("stream").and_then(Value::as_bool) == Some(true);
    if let Some(proof) = responses_mcp::reply(&payload) {
        match proof {
            Err(error) => response(StatusCode::BAD_REQUEST, Bytes::from(error), "text/plain"),
            Ok(message) if streaming => response(StatusCode::OK, responses_mcp::stream(message), "text/event-stream"),
            Ok(message) => json_response(message),
        }
    } else if streaming {
        response(
            StatusCode::OK,
            responses_stream(&payload, payload_has_function_call_output(&payload)),
            "text/event-stream",
        )
    } else {
        json_response(responses_response(payload))
    }
}

fn responses_response(payload: Value) -> Value {
    let model = payload.get("model").and_then(Value::as_str).unwrap_or("mock-local");
    let has_tool_output = serde_json::to_string(&payload)
        .map(|raw| raw.contains("function_call_output"))
        .unwrap_or(false);
    if !has_tool_output {
        return json!({
            "id": "resp_ironbank_tool_01",
            "object": "response",
            "created_at": 1781205836_u64,
            "status": "completed",
            "model": model,
            "output": [{
                "id": "fc_codex_write_poem",
                "type": "function_call",
                "status": "completed",
                "call_id": "call_codex_write_poem",
                "name": "exec_command",
                "arguments": "{\"cmd\":\"printf '%s\\\\n' 'Capsem ironbank poem\\\\nledgers count the sparks\\\\nno secret crosses raw' > /root/codex-cli-output.txt\",\"yield_time_ms\":1000,\"max_output_tokens\":2000}"
            }],
            "usage": {"input_tokens": 31, "output_tokens": 17, "total_tokens": 48}
        });
    }
    json!({
        "id": "resp_ironbank_01",
        "object": "response",
        "created_at": 1781205836_u64,
        "status": "completed",
        "model": model,
        "output": [{
            "id": "msg_ironbank_01",
            "type": "message",
            "status": "completed",
            "role": "assistant",
            "content": [{"type": "output_text", "text": EXPECTED_POEM, "annotations": []}]
        }],
        "output_text": EXPECTED_POEM,
        "usage": {
            "input_tokens": 7,
            "output_tokens": 5,
            "total_tokens": 12,
            "output_tokens_details": {"reasoning_tokens": 2}
        }
    })
}

fn payload_has_function_call_output(payload: &Value) -> bool {
    payload
        .get("input")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .any(|item| item.get("type").and_then(Value::as_str) == Some("function_call_output"))
        })
        .unwrap_or(false)
}

fn responses_stream(payload: &Value, final_turn: bool) -> Bytes {
    if final_turn {
        let (token, _) = write_target(payload, "openai-responses");
        return Bytes::from(
            format!(
                "event: response.reasoning_summary_text.delta\ndata: {{\"type\":\"response.reasoning_summary_text.delta\",\"delta\":\"ledger reasoning\"}}\n\n\
event: response.output_text.delta\ndata: {{\"type\":\"response.output_text.delta\",\"delta\":\"{token}\"}}\n\n\
event: response.output_text.done\ndata: {{\"type\":\"response.output_text.done\",\"text\":\"{token}\"}}\n\n\
event: response.completed\ndata: {{\"type\":\"response.completed\",\"response\":{{\"id\":\"resp_capsem_mock\",\"status\":\"completed\",\"model\":\"gpt-5-nano\",\"usage\":{{\"input_tokens\":7,\"output_tokens\":5,\"total_tokens\":12,\"output_tokens_details\":{{\"reasoning_tokens\":2}}}}}}}}\n\n"
            ),
        );
    }
    let (token, path) = write_target(payload, "openai-responses");
    let call_id = format!("call_{}", &token[..token.len().min(12)]);
    let arguments = json_compact(json!({
        "cmd": shell_write_command(&token, &path),
        "yield_time_ms": 1000,
        "max_output_tokens": 2000
    }));
    let arguments_json = json_compact(json!(arguments));
    Bytes::from(format!(
        "event: response.output_item.added\ndata: {{\"type\":\"response.output_item.added\",\"item\":{{\"type\":\"reasoning\",\"id\":\"rs_capsem_mock\"}}}}\n\n\
event: response.output_item.added\ndata: {{\"type\":\"response.output_item.added\",\"item\":{{\"type\":\"function_call\",\"id\":\"fc_capsem_mock\",\"call_id\":\"{call_id}\",\"name\":\"exec_command\",\"arguments\":{arguments_json}}}}}\n\n\
event: response.function_call_arguments.delta\ndata: {{\"type\":\"response.function_call_arguments.delta\",\"delta\":{arguments_json}}}\n\n\
event: response.function_call_arguments.done\ndata: {{\"type\":\"response.function_call_arguments.done\",\"arguments\":{arguments_json}}}\n\n\
event: response.output_item.done\ndata: {{\"type\":\"response.output_item.done\",\"item\":{{\"type\":\"function_call\",\"id\":\"fc_capsem_mock\",\"status\":\"completed\",\"call_id\":\"{call_id}\",\"name\":\"exec_command\",\"arguments\":{arguments_json}}}}}\n\n\
event: response.completed\ndata: {{\"type\":\"response.completed\",\"response\":{{\"id\":\"resp_capsem_mock\",\"status\":\"completed\",\"model\":\"gpt-5-nano\",\"usage\":{{\"input_tokens\":31,\"output_tokens\":17,\"total_tokens\":48,\"output_tokens_details\":{{\"reasoning_tokens\":2}}}}}}}}\n\n"
    ))
}
