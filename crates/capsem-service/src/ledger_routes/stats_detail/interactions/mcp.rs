//! Normalize observed MCP envelopes without confusing them with tool arguments.
use super::payload::{json_payload, preview_payload};
use super::ToolRow;
use capsem_api::*;
use serde::Deserialize;
use serde_json::value::RawValue;

#[derive(Deserialize)]
struct Request {
    jsonrpc: String,
    method: String,
    params: Params,
}
#[derive(Deserialize)]
struct Params {
    name: String,
    #[serde(default, deserialize_with = "present_json")]
    arguments: Option<Box<RawValue>>,
}
#[derive(Deserialize)]
struct Response {
    jsonrpc: String,
    #[serde(default, deserialize_with = "present_json")]
    result: Option<Box<RawValue>>,
    error: Option<RpcError>,
}
#[derive(Deserialize)]
struct RpcError {
    message: String,
}
#[derive(Deserialize)]
struct ResultFlags {
    #[serde(rename = "isError")]
    is_error: Option<bool>,
}

fn present_json<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<Box<RawValue>>, D::Error> {
    Box::<RawValue>::deserialize(deserializer).map(Some)
}

pub(super) fn tool_content(row: &ToolRow) -> InteractionToolCall {
    let mut call = InteractionToolCall {
        kind: InteractionToolCallKind::ToolCall,
        call_id: row.call_id.clone(),
        tool_name: row.tool_name.clone(),
        server_name: row.server_name.clone(),
        origin: row.origin,
        decision: row.decision,
        arguments: row
            .arguments
            .clone()
            .map(|raw| json_payload(raw, CaptureStatus::Unknown)),
        request: None,
        result: (row.response_preview.is_some() || row.error_message.is_some()).then(|| InteractionToolResult {
            kind: InteractionToolResultKind::ToolResult,
            call_id: row.call_id.clone(),
            payload: row.response_preview.clone().map(preview_payload),
            response: None,
            is_error: row.error_message.as_ref().map(|_| true),
            error_message: row.error_message.clone(),
        }),
    };
    if !matches!(row.origin, ToolOrigin::Mcp | ToolOrigin::McpProxy) || row.method.as_deref() != Some("tools/call") {
        return call;
    }
    if let Some(raw) = &row.arguments {
        if let Ok(request) = serde_json::from_str::<Request>(raw) {
            if request.jsonrpc == "2.0" && request.method == "tools/call" && request.params.name == row.tool_name {
                call.request = call.arguments.take();
                call.arguments = request
                    .params
                    .arguments
                    .map(|raw| json_payload(raw.get().to_owned(), CaptureStatus::Unknown));
            }
        }
    }
    if let (Some(raw), Some(result)) = (&row.response_preview, &mut call.result) {
        if let Ok(response) = serde_json::from_str::<Response>(raw) {
            if response.jsonrpc == "2.0" && (response.result.is_some() != response.error.is_some()) {
                result.response = result.payload.take();
                if let Some(value) = response.result {
                    result.is_error = serde_json::from_str::<ResultFlags>(value.get())
                        .ok()
                        .and_then(|flags| flags.is_error);
                    result.payload = Some(json_payload(value.get().to_owned(), CaptureStatus::Unknown));
                }
                if let Some(error) = response.error {
                    result.is_error = Some(true);
                    result.error_message = Some(error.message);
                }
            }
        }
    }
    call
}

#[cfg(test)]
mod tests;
