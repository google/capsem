//! Provider-agnostic LLM event types emitted by SSE stream parsers.
//!
//! Each AI provider (Anthropic, OpenAI, Google) has its own SSE wire format.
//! Provider-specific parsers convert those into these unified events, which
//! are then collected into a `StreamSummary` for audit logging.

use std::collections::BTreeMap;

use crate::net::parsers::sse_parser::SseEvent;

/// Why the model stopped generating.
#[derive(Debug, Clone, PartialEq)]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    ContentFilter,
    Other(String),
}

/// A single event from an LLM streaming response, provider-agnostic.
#[derive(Debug, Clone)]
pub enum LlmEvent {
    /// Stream started -- carries message ID and model name if available.
    MessageStart {
        message_id: Option<String>,
        model: Option<String>,
    },
    /// Incremental text output.
    TextDelta { index: u32, text: String },
    /// Incremental thinking/reasoning output.
    ThinkingDelta { index: u32, text: String },
    /// A tool call content block started.
    ToolCallStart {
        index: u32,
        call_id: String,
        name: String,
    },
    /// Incremental tool call arguments (JSON fragment).
    ToolCallArgumentDelta { index: u32, delta: String },
    /// A tool call content block finished.
    ToolCallEnd { index: u32 },
    /// A content block finished (text, thinking, or tool_use).
    ContentBlockEnd { index: u32 },
    /// Token usage update.
    Usage {
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        /// Breakdowns: e.g. {"cache_read": 800, "thinking": 200}
        details: BTreeMap<String, u64>,
    },
    /// Stream finished.
    MessageEnd { stop_reason: Option<StopReason> },
    /// Unrecognized event (logged but not parsed).
    Unknown {
        event_type: Option<String>,
        raw: String,
    },
}

/// A completed tool call extracted from the stream.
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub index: u32,
    pub call_id: String,
    pub name: String,
    pub arguments: String,
}

/// Summary of a complete LLM streaming response.
#[derive(Debug, Clone)]
pub struct StreamSummary {
    pub message_id: Option<String>,
    pub model: Option<String>,
    pub text: String,
    pub thinking: String,
    pub tool_calls: Vec<ToolCall>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub usage_details: BTreeMap<String, u64>,
    pub stop_reason: Option<StopReason>,
}

/// Summary extracted from a non-streaming model response body.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NonStreamingResponseSummary {
    pub text: String,
    pub thinking: String,
    pub stop_reason: Option<StopReason>,
}

/// Trait for provider-specific SSE-to-LlmEvent parsers.
///
/// Each provider implements this to convert their wire format
/// (already parsed into `SseEvent` by the SSE parser) into
/// unified `LlmEvent`s.
pub trait ProviderStreamParser: Send {
    fn parse_event(&mut self, sse: &SseEvent) -> Vec<LlmEvent>;
}

/// Collect a sequence of `LlmEvent`s into a `StreamSummary`.
///
/// Pure function -- no I/O. Concatenates text deltas, builds tool calls
/// from start/delta/end sequences, captures the last usage and stop reason.
pub fn collect_summary(events: &[LlmEvent]) -> StreamSummary {
    let mut message_id: Option<String> = None;
    let mut model: Option<String> = None;
    let mut text = String::new();
    let mut thinking = String::new();
    let mut input_tokens: Option<u64> = None;
    let mut output_tokens: Option<u64> = None;
    let mut usage_details: BTreeMap<String, u64> = BTreeMap::new();
    let mut stop_reason: Option<StopReason> = None;

    // In-progress tool calls keyed by content block index.
    let mut builders: Vec<(u32, String, String, String)> = Vec::new(); // (index, call_id, name, args)
    let mut completed: Vec<ToolCall> = Vec::new();

    for event in events {
        match event {
            LlmEvent::MessageStart {
                message_id: mid,
                model: m,
            } => {
                if mid.is_some() {
                    message_id = mid.clone();
                }
                if m.is_some() {
                    model = m.clone();
                }
            }
            LlmEvent::TextDelta { text: t, .. } => {
                text.push_str(t);
            }
            LlmEvent::ThinkingDelta { text: t, .. } => {
                thinking.push_str(t);
            }
            LlmEvent::ToolCallStart {
                index,
                call_id,
                name,
            } => {
                builders.push((*index, call_id.clone(), name.clone(), String::new()));
            }
            LlmEvent::ToolCallArgumentDelta { index, delta } => {
                // Find the builder for this index (most recent with matching index)
                for (idx, _, _, args) in builders.iter_mut().rev() {
                    if *idx == *index {
                        args.push_str(delta);
                        break;
                    }
                }
            }
            LlmEvent::ToolCallEnd { index } => {
                // Move the builder to completed
                if let Some(pos) = builders.iter().rposition(|(idx, _, _, _)| *idx == *index) {
                    let (idx, call_id, name, arguments) = builders.remove(pos);
                    completed.push(ToolCall {
                        index: idx,
                        call_id,
                        name,
                        arguments,
                    });
                }
            }
            LlmEvent::ContentBlockEnd { index } => {
                // Also flushes tool calls that ended via ContentBlockEnd
                if let Some(pos) = builders.iter().rposition(|(idx, _, _, _)| *idx == *index) {
                    let (idx, call_id, name, arguments) = builders.remove(pos);
                    completed.push(ToolCall {
                        index: idx,
                        call_id,
                        name,
                        arguments,
                    });
                }
            }
            LlmEvent::Usage {
                input_tokens: it,
                output_tokens: ot,
                details,
            } => {
                if let Some(t) = it {
                    input_tokens = Some(*t);
                }
                if let Some(t) = ot {
                    output_tokens = Some(*t);
                }
                for (k, v) in details {
                    usage_details.insert(k.clone(), *v);
                }
            }
            LlmEvent::MessageEnd { stop_reason: sr } => {
                stop_reason = sr.clone();
            }
            LlmEvent::Unknown { .. } => {}
        }
    }

    // Flush any tool calls that were never explicitly ended
    for (idx, call_id, name, arguments) in builders {
        completed.push(ToolCall {
            index: idx,
            call_id,
            name,
            arguments,
        });
    }
    completed.sort_by_key(|tc| tc.index);

    StreamSummary {
        message_id,
        model,
        text,
        thinking,
        tool_calls: completed,
        input_tokens,
        output_tokens,
        usage_details,
        stop_reason,
    }
}

/// Parse usage metadata from a non-streaming JSON response body.
/// Handles gzip-compressed responses (common when upstream sends
/// Content-Encoding: gzip through the MITM proxy).
/// Returns (model, input_tokens, output_tokens, usage_details).
pub fn parse_non_streaming_usage(
    kind: super::provider::ModelProtocol,
    body: &[u8],
) -> (
    Option<String>,
    Option<u64>,
    Option<u64>,
    BTreeMap<String, u64>,
) {
    let Some(json) = parse_response_json(body) else {
        return (None, None, None, BTreeMap::new());
    };

    match kind {
        super::provider::ModelProtocol::Google => {
            let json = google_response_envelope(&json);
            let model = json
                .get("modelVersion")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let usage = json.get("usageMetadata");
            let input = usage
                .and_then(|u| u.get("promptTokenCount"))
                .and_then(|v| v.as_u64());
            let output = usage
                .and_then(|u| u.get("candidatesTokenCount"))
                .and_then(|v| v.as_u64());
            let mut details = BTreeMap::new();
            if let Some(v) = usage
                .and_then(|u| u.get("cachedContentTokenCount"))
                .and_then(|v| v.as_u64())
            {
                details.insert("cache_read".into(), v);
            }
            if let Some(v) = usage
                .and_then(|u| u.get("thoughtsTokenCount"))
                .and_then(|v| v.as_u64())
            {
                details.insert("thinking".into(), v);
            }
            (model, input, output, details)
        }
        super::provider::ModelProtocol::Anthropic => {
            let model = json
                .get("model")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let usage = json.get("usage");
            let input = usage
                .and_then(|u| u.get("input_tokens"))
                .and_then(|v| v.as_u64());
            let output = usage
                .and_then(|u| u.get("output_tokens"))
                .and_then(|v| v.as_u64());
            let mut details = BTreeMap::new();
            if let Some(v) = usage
                .and_then(|u| u.get("cache_read_input_tokens"))
                .and_then(|v| v.as_u64())
            {
                details.insert("cache_read".into(), v);
            }
            (model, input, output, details)
        }
        super::provider::ModelProtocol::OpenAi => {
            let model = json
                .get("model")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let usage = json.get("usage");
            let input = usage.and_then(|u| {
                u.get("prompt_tokens")
                    .or_else(|| u.get("input_tokens"))
                    .and_then(|v| v.as_u64())
            });
            let output = usage.and_then(|u| {
                u.get("completion_tokens")
                    .or_else(|| u.get("output_tokens"))
                    .and_then(|v| v.as_u64())
            });
            let mut details = BTreeMap::new();
            if let Some(v) = usage
                .and_then(|u| u.get("prompt_tokens_details"))
                .or_else(|| usage.and_then(|u| u.get("input_tokens_details")))
                .and_then(|u| u.get("cached_tokens"))
                .and_then(|v| v.as_u64())
            {
                details.insert("cache_read".into(), v);
            }
            if let Some(v) = usage
                .and_then(|u| u.get("completion_tokens_details"))
                .or_else(|| usage.and_then(|u| u.get("output_tokens_details")))
                .and_then(|u| u.get("reasoning_tokens"))
                .and_then(|v| v.as_u64())
            {
                details.insert("thinking".into(), v);
            }
            (model, input, output, details)
        }
        super::provider::ModelProtocol::Ollama => {
            let model = json
                .get("model")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let input = json.get("prompt_eval_count").and_then(|v| v.as_u64());
            let output = json.get("eval_count").and_then(|v| v.as_u64());
            (model, input, output, BTreeMap::new())
        }
    }
}

/// Parse model-native tool calls from a non-streaming JSON response body.
pub fn parse_non_streaming_tool_calls(
    kind: super::provider::ModelProtocol,
    body: &[u8],
) -> Vec<ToolCall> {
    let Some(json) = parse_response_json(body) else {
        return Vec::new();
    };
    match kind {
        super::provider::ModelProtocol::Google => {
            google_non_streaming_tool_calls(google_response_envelope(&json))
        }
        super::provider::ModelProtocol::OpenAi => openai_non_streaming_tool_calls(&json),
        super::provider::ModelProtocol::Anthropic => anthropic_non_streaming_tool_calls(&json),
        _ => Vec::new(),
    }
}

/// Parse assistant text, thinking, and stop reason from a non-streaming JSON
/// response body. This mirrors streaming `LlmEvent` collection so model
/// ledgers do not lose content when a provider returns a complete JSON body.
pub fn parse_non_streaming_response_summary(
    kind: super::provider::ModelProtocol,
    body: &[u8],
) -> NonStreamingResponseSummary {
    let Some(json) = parse_response_json(body) else {
        return NonStreamingResponseSummary::default();
    };
    match kind {
        super::provider::ModelProtocol::OpenAi => openai_non_streaming_response_summary(&json),
        super::provider::ModelProtocol::Anthropic => {
            anthropic_non_streaming_response_summary(&json)
        }
        super::provider::ModelProtocol::Google => {
            google_non_streaming_response_summary(google_response_envelope(&json))
        }
        super::provider::ModelProtocol::Ollama => ollama_non_streaming_response_summary(&json),
    }
}

fn google_response_envelope(json: &serde_json::Value) -> &serde_json::Value {
    json.get("response")
        .filter(|response| response.is_object())
        .unwrap_or(json)
}

fn parse_response_json(body: &[u8]) -> Option<serde_json::Value> {
    if let Ok(v) = serde_json::from_slice(body) {
        return Some(v);
    }
    if body.len() >= 2 && body[0] == 0x1f && body[1] == 0x8b {
        use flate2::read::GzDecoder;
        use std::io::Read;
        let mut decoder = GzDecoder::new(body);
        let mut decompressed = Vec::new();
        if decoder.read_to_end(&mut decompressed).is_err() {
            return None;
        }
        return serde_json::from_slice(&decompressed).ok();
    }
    None
}

fn google_non_streaming_tool_calls(json: &serde_json::Value) -> Vec<ToolCall> {
    let mut calls = Vec::new();
    let Some(candidates) = json.get("candidates").and_then(|value| value.as_array()) else {
        return calls;
    };
    for candidate in candidates {
        let Some(parts) = candidate
            .get("content")
            .and_then(|content| content.get("parts"))
            .and_then(|parts| parts.as_array())
        else {
            continue;
        };
        for part in parts {
            let Some(function_call) = part.get("functionCall") else {
                continue;
            };
            let name = function_call
                .get("name")
                .and_then(|name| name.as_str())
                .unwrap_or_default()
                .to_string();
            let args = function_call
                .get("args")
                .map(|args| serde_json::to_string(args).unwrap_or_else(|_| "{}".to_string()))
                .unwrap_or_else(|| "{}".to_string());
            let index = calls.len() as u32;
            let call_id = function_call
                .get("id")
                .and_then(|id| id.as_str())
                .map(str::to_string)
                .filter(|id| !id.is_empty())
                .unwrap_or_else(|| format!("gemini_{}_{}", name, index));
            calls.push(ToolCall {
                index,
                call_id,
                name,
                arguments: args,
            });
        }
    }
    calls
}

fn anthropic_non_streaming_tool_calls(json: &serde_json::Value) -> Vec<ToolCall> {
    let mut calls = Vec::new();
    let Some(content) = json.get("content").and_then(|value| value.as_array()) else {
        return calls;
    };
    for part in content {
        if part.get("type").and_then(|value| value.as_str()) != Some("tool_use") {
            continue;
        }
        let name = part
            .get("name")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        if name.is_empty() {
            continue;
        }
        let index = calls.len() as u32;
        let call_id = part
            .get("id")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| format!("anthropic_{name}_{index}"));
        let arguments = part
            .get("input")
            .map(|value| serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string()))
            .unwrap_or_else(|| "{}".to_string());
        calls.push(ToolCall {
            index,
            call_id,
            name,
            arguments,
        });
    }
    calls
}

fn openai_non_streaming_response_summary(json: &serde_json::Value) -> NonStreamingResponseSummary {
    let mut summary = NonStreamingResponseSummary::default();
    if let Some(data) = json.get("data").and_then(|value| value.as_array()) {
        for item in data {
            append_json_string(&mut summary.text, item.get("b64_json"));
            append_json_string(&mut summary.text, item.get("url"));
        }
        if !summary.text.is_empty() {
            summary.stop_reason = Some(StopReason::EndTurn);
            return summary;
        }
    }
    if json.get("object").and_then(|value| value.as_str()) == Some("response") {
        if json
            .get("status")
            .and_then(|value| value.as_str())
            .is_some_and(|status| status == "completed")
        {
            summary.stop_reason = Some(StopReason::EndTurn);
        }
        if let Some(output) = json.get("output").and_then(|value| value.as_array()) {
            for item in output {
                match item.get("type").and_then(|value| value.as_str()) {
                    Some("message") => {
                        if let Some(content) =
                            item.get("content").and_then(|value| value.as_array())
                        {
                            for part in content {
                                append_openai_content(&mut summary.text, Some(part));
                            }
                        }
                    }
                    Some("reasoning") => {
                        if let Some(summary_parts) =
                            item.get("summary").and_then(|value| value.as_array())
                        {
                            for part in summary_parts {
                                append_openai_content(&mut summary.thinking, Some(part));
                            }
                        }
                        if let Some(content) =
                            item.get("content").and_then(|value| value.as_array())
                        {
                            for part in content {
                                append_openai_content(&mut summary.thinking, Some(part));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        return summary;
    }
    let Some(choices) = json.get("choices").and_then(|value| value.as_array()) else {
        return summary;
    };
    for choice in choices {
        if let Some(reason) = choice.get("finish_reason").and_then(|value| value.as_str()) {
            summary.stop_reason = Some(stop_reason_from_provider_string(reason));
        }
        if let Some(message) = choice.get("message") {
            append_openai_content(&mut summary.text, message.get("content"));
            append_openai_content(&mut summary.thinking, message.get("reasoning_content"));
            append_openai_content(&mut summary.thinking, message.get("thinking"));
        }
    }
    summary
}

fn anthropic_non_streaming_response_summary(
    json: &serde_json::Value,
) -> NonStreamingResponseSummary {
    let mut summary = NonStreamingResponseSummary {
        stop_reason: json
            .get("stop_reason")
            .and_then(|value| value.as_str())
            .map(stop_reason_from_provider_string),
        ..Default::default()
    };
    let Some(content) = json.get("content").and_then(|value| value.as_array()) else {
        return summary;
    };
    for part in content {
        match part.get("type").and_then(|value| value.as_str()) {
            Some("text") => {
                append_json_string(&mut summary.text, part.get("text"));
            }
            Some("thinking") | Some("reasoning") => {
                append_json_string(&mut summary.thinking, part.get("thinking"));
                append_json_string(&mut summary.thinking, part.get("text"));
            }
            _ => {}
        }
    }
    summary
}

fn google_non_streaming_response_summary(json: &serde_json::Value) -> NonStreamingResponseSummary {
    let mut summary = NonStreamingResponseSummary::default();
    let Some(candidates) = json.get("candidates").and_then(|value| value.as_array()) else {
        return summary;
    };
    for candidate in candidates {
        if let Some(reason) = candidate
            .get("finishReason")
            .and_then(|value| value.as_str())
        {
            summary.stop_reason = Some(stop_reason_from_provider_string(reason));
        }
        let Some(parts) = candidate
            .get("content")
            .and_then(|content| content.get("parts"))
            .and_then(|parts| parts.as_array())
        else {
            continue;
        };
        for part in parts {
            append_json_string(&mut summary.text, part.get("text"));
            append_json_string(&mut summary.thinking, part.get("thought"));
            append_json_string(&mut summary.thinking, part.get("thinking"));
        }
    }
    summary
}

fn ollama_non_streaming_response_summary(json: &serde_json::Value) -> NonStreamingResponseSummary {
    let mut summary = NonStreamingResponseSummary::default();
    append_json_string(&mut summary.text, json.get("response"));
    if let Some(message) = json.get("message") {
        append_json_string(&mut summary.text, message.get("content"));
        append_json_string(&mut summary.thinking, message.get("thinking"));
    }
    if json
        .get("done")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        summary.stop_reason = Some(StopReason::EndTurn);
    }
    summary
}

fn append_openai_content(target: &mut String, value: Option<&serde_json::Value>) {
    let Some(value) = value else {
        return;
    };
    if append_json_string(target, Some(value)) {
        return;
    }
    if let Some(part_type) = value.get("type").and_then(|value| value.as_str()) {
        match part_type {
            "text" | "output_text" | "summary_text" => {
                append_json_string(target, value.get("text"));
            }
            _ => {}
        }
        return;
    }
    let Some(parts) = value.as_array() else {
        return;
    };
    for part in parts {
        match part.get("type").and_then(|value| value.as_str()) {
            Some("text") | Some("output_text") | Some("summary_text") => {
                append_json_string(target, part.get("text"));
            }
            _ => {}
        }
    }
}

fn append_json_string(target: &mut String, value: Option<&serde_json::Value>) -> bool {
    let Some(text) = value.and_then(|value| value.as_str()) else {
        return false;
    };
    if !target.is_empty() && !text.is_empty() {
        target.push('\n');
    }
    target.push_str(text);
    true
}

fn stop_reason_from_provider_string(reason: &str) -> StopReason {
    match reason {
        "end_turn" | "stop" | "STOP" => StopReason::EndTurn,
        "tool_use" | "tool_calls" | "function_call" => StopReason::ToolUse,
        "max_tokens" | "length" | "MAX_TOKENS" => StopReason::MaxTokens,
        "content_filter" | "SAFETY" | "RECITATION" => StopReason::ContentFilter,
        other => StopReason::Other(other.to_string()),
    }
}

fn openai_non_streaming_tool_calls(json: &serde_json::Value) -> Vec<ToolCall> {
    let mut calls = Vec::new();
    if json.get("object").and_then(|value| value.as_str()) == Some("response") {
        if let Some(output) = json.get("output").and_then(|value| value.as_array()) {
            for item in output {
                if item.get("type").and_then(|value| value.as_str()) != Some("function_call") {
                    continue;
                }
                let index = calls.len() as u32;
                let name = item
                    .get("name")
                    .and_then(|name| name.as_str())
                    .unwrap_or_default()
                    .to_string();
                if name.is_empty() {
                    continue;
                }
                let call_id = item
                    .get("call_id")
                    .or_else(|| item.get("id"))
                    .and_then(|id| id.as_str())
                    .map(str::to_string)
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| format!("openai_{}_{}", name, index));
                let arguments = item
                    .get("arguments")
                    .and_then(|arguments| arguments.as_str())
                    .map(str::to_string)
                    .unwrap_or_else(|| "{}".to_string());
                calls.push(ToolCall {
                    index,
                    call_id,
                    name,
                    arguments,
                });
            }
        }
        return calls;
    }
    let Some(choices) = json.get("choices").and_then(|value| value.as_array()) else {
        return calls;
    };
    for choice in choices {
        let Some(tool_calls) = choice
            .get("message")
            .and_then(|message| message.get("tool_calls"))
            .and_then(|tool_calls| tool_calls.as_array())
        else {
            continue;
        };
        for tool_call in tool_calls {
            let index = tool_call
                .get("index")
                .and_then(|index| index.as_u64())
                .map(|index| index as u32)
                .unwrap_or(calls.len() as u32);
            let call_id = tool_call
                .get("id")
                .and_then(|id| id.as_str())
                .unwrap_or_default()
                .to_string();
            let Some(function) = tool_call.get("function") else {
                continue;
            };
            let name = function
                .get("name")
                .and_then(|name| name.as_str())
                .unwrap_or_default()
                .to_string();
            if name.is_empty() {
                continue;
            }
            let arguments = function
                .get("arguments")
                .and_then(|arguments| arguments.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| "{}".to_string());
            calls.push(ToolCall {
                index,
                call_id: if call_id.is_empty() {
                    format!("openai_{}_{}", name, index)
                } else {
                    call_id
                },
                name,
                arguments,
            });
        }
    }
    calls.sort_by_key(|call| call.index);
    calls
}

#[cfg(test)]
mod tests;
