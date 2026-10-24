//! Existing Google Code Assist fixture and stream owner.
use super::{json_compact, json_response, response, shell_write_command, target_dir, write_target, RespBody};
use anyhow::{Context, Result};
use bytes::Bytes;
use hyper::{Response, StatusCode};
use serde_json::{json, Value};

pub(super) fn reply(payload: Value) -> Response<RespBody> {
    let body = match super::google_mcp::reply(&payload) {
        None => google_code_assist_stream(payload),
        Some(Ok(body)) => body,
        Some(Err(error)) => return response(StatusCode::BAD_REQUEST, Bytes::from(error), "text/plain"),
    };
    response(StatusCode::OK, body, "text/event-stream")
}

const GOOGLE_FIXTURES: &[(&str, &str)] = &[
    (
        "available_models.json",
        include_str!("../../../tests/fixtures/protocols/google_code_assist/available_models.json"),
    ),
    (
        "list_experiments.json",
        include_str!("../../../tests/fixtures/protocols/google_code_assist/list_experiments.json"),
    ),
    (
        "load_code_assist.json",
        include_str!("../../../tests/fixtures/protocols/google_code_assist/load_code_assist.json"),
    ),
    (
        "quota_summary.json",
        include_str!("../../../tests/fixtures/protocols/google_code_assist/quota_summary.json"),
    ),
];

pub(super) fn google_fixture_response(name: &str) -> Response<RespBody> {
    // A fixture that will not parse is a defect in this repository, not an
    // upstream condition worth simulating. Answering 500 with the reason keeps
    // the failure where it belongs instead of handing the client a plausible
    // empty catalog to be confused by later.
    match load_google_fixture(name) {
        Ok(value) => json_response(value),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            Bytes::from(format!("mock fixture unavailable: {error:#}")),
            "text/plain",
        ),
    }
}

fn load_google_fixture(name: &str) -> Result<Value> {
    let text = GOOGLE_FIXTURES
        .iter()
        .find_map(|(fixture, text)| (*fixture == name).then_some(*text))
        .with_context(|| format!("unknown google code-assist fixture: {name}"))?;
    serde_json::from_str(text).with_context(|| format!("parse {name}"))
}

pub(super) fn google_code_assist_stream(payload: Value) -> Bytes {
    if payload.get("requestType").and_then(Value::as_str) == Some("checkpoint") {
        let model = payload
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("gemini-3.1-flash-lite");
        let response = json!({
            "response": {
                "candidates": [{
                    "content": {"parts": [{"text": "Write Proof"}], "role": "model"},
                    "finishReason": "STOP"
                }],
                "modelVersion": model,
                "responseId": "agy_checkpoint"
            },
            "traceId": "trace_checkpoint",
            "metadata": {}
        });
        return Bytes::from(format!("data: {}\n\n", json_compact(response)));
    }

    let (token, path) = write_target(&payload, "agy");
    let call_id = format!("call_{}", &token[..token.len().min(12)]);
    let response_id = format!("agy_{}", &token[..token.len().min(12)]);
    let trace_id = format!("trace_{}", &token[..token.len().min(12)]);
    let has_function_response = serde_json::to_string(&payload)
        .map(|raw| raw.contains("functionResponse"))
        .unwrap_or(false);
    if has_function_response {
        let final_response = json!({
            "response": {
                "candidates": [{
                    "content": {"parts": [{"text": token}], "role": "model"},
                    "finishReason": "STOP"
                }],
                "usageMetadata": {
                    "promptTokenCount": 31,
                    "candidatesTokenCount": 17,
                    "thoughtsTokenCount": 2,
                    "totalTokenCount": 50
                },
                "modelVersion": "gemini-3.5-flash-low",
                "responseId": response_id
            },
            "traceId": trace_id,
            "metadata": {}
        });
        return Bytes::from(format!("data: {}\n\n", json_compact(final_response)));
    }
    let first = json!({
        "response": {
            "candidates": [{
                "content": {
                    "parts": [{
                        "thoughtSignature": "capsem-agy-fixture-signature",
                        "functionCall": {
                            "name": "run_command",
                            "id": call_id,
                            "args": {
                                "CommandLine": shell_write_command(&token, &path),
                                "Cwd": target_dir(&path),
                                "WaitMsBeforeAsync": 1000,
                                "toolSummary": "Write proof",
                                "toolAction": "Writing file"
                            }
                        }
                    }],
                    "role": "model"
                }
            }],
            "usageMetadata": {
                "promptTokenCount": 31,
                "candidatesTokenCount": 17,
                "thoughtsTokenCount": 2,
                "totalTokenCount": 50
            },
            "modelVersion": "gemini-3.5-flash-low",
            "responseId": response_id
        },
        "traceId": trace_id,
        "metadata": {}
    });
    let final_chunk = json!({
        "response": {
            "candidates": [{
                "content": {"parts": [{"text": ""}], "role": "model"},
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 31,
                "candidatesTokenCount": 17,
                "thoughtsTokenCount": 2,
                "totalTokenCount": 50
            },
            "modelVersion": "gemini-3.5-flash-low",
            "responseId": first["response"]["responseId"].clone()
        },
        "traceId": first["traceId"].clone(),
        "metadata": {}
    });
    Bytes::from(format!(
        "data: {}\n\ndata: {}\n\n",
        json_compact(first),
        json_compact(final_chunk)
    ))
}
