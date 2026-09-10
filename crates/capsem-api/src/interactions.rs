//! Shared inspection objects derived from retained model/MCP ledger evidence.
//! Preview captures do not imply complete messages or a complete conversation.
use crate::{BodyDirection, ToolDecision, ToolOrigin};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CaptureStatus {
    Complete,
    Truncated,
    /// The ledger retained a preview without recording its original length.
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CapturedPayload {
    pub status: CaptureStatus,
    pub content: CapturedContent,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum CapturedContent {
    Json(JsonContent),
    Text(TextContent),
    Raw(RawContent),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum JsonContentKind {
    Json,
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct JsonContent {
    pub kind: JsonContentKind,
    /// Arbitrary tool/provider JSON, including arrays, scalars and JSON null.
    #[serde(deserialize_with = "deserialize_json_value")]
    pub value: serde_json::Value,
}

fn deserialize_json_value<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<serde_json::Value, D::Error> {
    serde_json::Value::deserialize(deserializer)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextContentKind {
    Text,
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TextContent {
    pub kind: TextContentKind,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RawContentKind {
    Raw,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RawContentReason {
    Truncated,
    InvalidJson,
    Unparsed,
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RawContent {
    pub kind: RawContentKind,
    pub raw: String,
    pub reason: RawContentReason,
}

/// One retained item. IDs refer to the session ledger, never a list position.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Interaction {
    pub event_id: String,
    pub timestamp: String,
    pub model_call_id: Option<i64>,
    /// Null when there is no parent or its row is no longer retained.
    pub model_event_id: Option<String>,
    pub trace_id: Option<String>,
    pub turn_id: Option<String>,
    pub item_index: Option<u64>,
    pub content: InteractionContent,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum InteractionContent {
    Request(InteractionRequest),
    Message(InteractionMessage),
    ToolCall(InteractionToolCall),
    ToolResult(InteractionToolResult),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum InteractionRequestKind {
    RequestPreview,
}
/// A retained request preview, not an inferred user message.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct InteractionRequest {
    pub kind: InteractionRequestKind,
    pub payload: Option<CapturedPayload>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum InteractionMessageKind {
    Message,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum InteractionRole {
    Assistant,
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct InteractionMessage {
    pub kind: InteractionMessageKind,
    pub role: InteractionRole,
    pub blocks: Vec<InteractionBlock>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum InteractionBlockKind {
    Text,
    Reasoning,
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct InteractionBlock {
    pub kind: InteractionBlockKind,
    pub payload: Option<CapturedPayload>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum InteractionToolCallKind {
    ToolCall,
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct InteractionToolCall {
    pub kind: InteractionToolCallKind,
    /// Provider/server scoped: use the enclosing trace and model IDs to correlate.
    pub call_id: String,
    pub tool_name: String,
    pub server_name: Option<String>,
    pub origin: ToolOrigin,
    pub decision: ToolDecision,
    pub arguments: Option<CapturedPayload>,
    /// Captured JSON-RPC request when the ledger stored an MCP envelope.
    pub request: Option<CapturedPayload>,
    /// Only the result stored on this call; unrelated call IDs are never joined.
    pub result: Option<InteractionToolResult>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum InteractionToolResultKind {
    ToolResult,
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct InteractionToolResult {
    pub kind: InteractionToolResultKind,
    pub call_id: String,
    pub payload: Option<CapturedPayload>,
    /// Captured JSON-RPC response when `payload` is its extracted result.
    pub response: Option<CapturedPayload>,
    /// Null when the source ledger did not record a result error flag.
    pub is_error: Option<bool>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct InteractionBody {
    pub event_id: String,
    pub direction: BodyDirection,
    pub content_type: Option<String>,
    pub original_bytes: u64,
    pub stored_bytes: u64,
    pub body_hash: String,
    pub payload: CapturedPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct InteractionReport {
    /// Latest 200 model items and latest 200 tool calls, in chronological order.
    /// This bounded report is not a complete conversation or a pagination API.
    pub items: Vec<Interaction>,
    pub bodies: Vec<InteractionBody>,
}

#[cfg(test)]
mod tests;
