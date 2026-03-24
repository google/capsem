//! Provider-agnostic LLM event types emitted by SSE stream parsers.
//!
//! Each AI provider (Anthropic, OpenAI, Google) has its own SSE wire format.
//! Provider-specific parsers convert those into these unified events, which
//! are then collected into a `StreamSummary` for audit logging.

use std::collections::BTreeMap;

use super::sse::SseEvent;

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
    MessageEnd {
        stop_reason: Option<StopReason>,
    },
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
            LlmEvent::MessageStart { message_id: mid, model: m } => {
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
            LlmEvent::ToolCallStart { index, call_id, name } => {
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
                    completed.push(ToolCall { index: idx, call_id, name, arguments });
                }
            }
            LlmEvent::ContentBlockEnd { index } => {
                // Also flushes tool calls that ended via ContentBlockEnd
                if let Some(pos) = builders.iter().rposition(|(idx, _, _, _)| *idx == *index) {
                    let (idx, call_id, name, arguments) = builders.remove(pos);
                    completed.push(ToolCall { index: idx, call_id, name, arguments });
                }
            }
            LlmEvent::Usage { input_tokens: it, output_tokens: ot, details } => {
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
        completed.push(ToolCall { index: idx, call_id, name, arguments });
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
    kind: super::provider::ProviderKind,
    body: &[u8],
) -> (Option<String>, Option<u64>, Option<u64>, BTreeMap<String, u64>) {
    // Try plain JSON first, then gzip-decompress if it fails.
    let json: serde_json::Value = if let Ok(v) = serde_json::from_slice(body) {
        v
    } else if body.len() >= 2 && body[0] == 0x1f && body[1] == 0x8b {
        // Gzip magic bytes -- decompress and retry.
        use flate2::read::GzDecoder;
        use std::io::Read;
        let mut decoder = GzDecoder::new(body);
        let mut decompressed = Vec::new();
        if decoder.read_to_end(&mut decompressed).is_err() {
            return (None, None, None, BTreeMap::new());
        }
        match serde_json::from_slice(&decompressed) {
            Ok(v) => v,
            Err(_) => return (None, None, None, BTreeMap::new()),
        }
    } else {
        return (None, None, None, BTreeMap::new());
    };

    match kind {
        super::provider::ProviderKind::Google => {
            let model = json.get("modelVersion")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let usage = json.get("usageMetadata");
            let input = usage.and_then(|u| u.get("promptTokenCount")).and_then(|v| v.as_u64());
            let output = usage.and_then(|u| u.get("candidatesTokenCount")).and_then(|v| v.as_u64());
            let mut details = BTreeMap::new();
            if let Some(v) = usage.and_then(|u| u.get("cachedContentTokenCount")).and_then(|v| v.as_u64()) {
                details.insert("cache_read".into(), v);
            }
            if let Some(v) = usage.and_then(|u| u.get("thoughtsTokenCount")).and_then(|v| v.as_u64()) {
                details.insert("thinking".into(), v);
            }
            (model, input, output, details)
        }
        super::provider::ProviderKind::Anthropic => {
            let model = json.get("model").and_then(|v| v.as_str()).map(|s| s.to_string());
            let usage = json.get("usage");
            let input = usage.and_then(|u| u.get("input_tokens")).and_then(|v| v.as_u64());
            let output = usage.and_then(|u| u.get("output_tokens")).and_then(|v| v.as_u64());
            let mut details = BTreeMap::new();
            if let Some(v) = usage.and_then(|u| u.get("cache_read_input_tokens")).and_then(|v| v.as_u64()) {
                details.insert("cache_read".into(), v);
            }
            (model, input, output, details)
        }
        super::provider::ProviderKind::OpenAi => {
            let model = json.get("model").and_then(|v| v.as_str()).map(|s| s.to_string());
            let usage = json.get("usage");
            let input = usage.and_then(|u| u.get("prompt_tokens")).and_then(|v| v.as_u64());
            let output = usage.and_then(|u| u.get("completion_tokens")).and_then(|v| v.as_u64());
            let mut details = BTreeMap::new();
            if let Some(v) = usage.and_then(|u| u.get("prompt_tokens_details")).and_then(|u| u.get("cached_tokens")).and_then(|v| v.as_u64()) {
                details.insert("cache_read".into(), v);
            }
            if let Some(v) = usage.and_then(|u| u.get("completion_tokens_details")).and_then(|u| u.get("reasoning_tokens")).and_then(|v| v.as_u64()) {
                details.insert("thinking".into(), v);
            }
            (model, input, output, details)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── collect_summary: text-only stream ───────────────────────────

    #[test]
    fn summary_text_only() {
        let events = vec![
            LlmEvent::MessageStart {
                message_id: Some("msg_01".into()),
                model: Some("claude-sonnet-4-20250514".into()),
            },
            LlmEvent::TextDelta { index: 0, text: "Hello".into() },
            LlmEvent::TextDelta { index: 0, text: " world".into() },
            LlmEvent::Usage { input_tokens: Some(10), output_tokens: Some(5), details: BTreeMap::new() },
            LlmEvent::MessageEnd { stop_reason: Some(StopReason::EndTurn) },
        ];

        let s = collect_summary(&events);
        assert_eq!(s.message_id.as_deref(), Some("msg_01"));
        assert_eq!(s.model.as_deref(), Some("claude-sonnet-4-20250514"));
        assert_eq!(s.text, "Hello world");
        assert!(s.thinking.is_empty());
        assert!(s.tool_calls.is_empty());
        assert_eq!(s.input_tokens, Some(10));
        assert_eq!(s.output_tokens, Some(5));
        assert_eq!(s.stop_reason, Some(StopReason::EndTurn));
    }

    // ── collect_summary: tool calls ─────────────────────────────────

    #[test]
    fn summary_tool_calls() {
        let events = vec![
            LlmEvent::MessageStart { message_id: None, model: None },
            LlmEvent::ToolCallStart {
                index: 0,
                call_id: "call_1".into(),
                name: "get_weather".into(),
            },
            LlmEvent::ToolCallArgumentDelta { index: 0, delta: r#"{"loc"#.into() },
            LlmEvent::ToolCallArgumentDelta { index: 0, delta: r#"ation":"NYC"}"#.into() },
            LlmEvent::ToolCallEnd { index: 0 },
            LlmEvent::MessageEnd { stop_reason: Some(StopReason::ToolUse) },
        ];

        let s = collect_summary(&events);
        assert_eq!(s.tool_calls.len(), 1);
        assert_eq!(s.tool_calls[0].call_id, "call_1");
        assert_eq!(s.tool_calls[0].name, "get_weather");
        assert_eq!(s.tool_calls[0].arguments, r#"{"location":"NYC"}"#);
        assert_eq!(s.stop_reason, Some(StopReason::ToolUse));
    }

    // ── collect_summary: mixed text + tool calls ────────────────────

    #[test]
    fn summary_mixed_content() {
        let events = vec![
            LlmEvent::MessageStart { message_id: Some("msg_02".into()), model: None },
            LlmEvent::TextDelta { index: 0, text: "Let me check ".into() },
            LlmEvent::TextDelta { index: 0, text: "the weather.".into() },
            LlmEvent::ContentBlockEnd { index: 0 },
            LlmEvent::ToolCallStart {
                index: 1,
                call_id: "call_x".into(),
                name: "weather".into(),
            },
            LlmEvent::ToolCallArgumentDelta { index: 1, delta: "{}".into() },
            LlmEvent::ToolCallEnd { index: 1 },
            LlmEvent::Usage { input_tokens: Some(20), output_tokens: Some(15), details: BTreeMap::new() },
            LlmEvent::MessageEnd { stop_reason: Some(StopReason::ToolUse) },
        ];

        let s = collect_summary(&events);
        assert_eq!(s.text, "Let me check the weather.");
        assert_eq!(s.tool_calls.len(), 1);
        assert_eq!(s.tool_calls[0].index, 1);
    }

    // ── collect_summary: thinking ───────────────────────────────────

    #[test]
    fn summary_with_thinking() {
        let events = vec![
            LlmEvent::MessageStart { message_id: None, model: None },
            LlmEvent::ThinkingDelta { index: 0, text: "Let me think".into() },
            LlmEvent::ThinkingDelta { index: 0, text: " about this.".into() },
            LlmEvent::ContentBlockEnd { index: 0 },
            LlmEvent::TextDelta { index: 1, text: "Here's my answer.".into() },
            LlmEvent::ContentBlockEnd { index: 1 },
            LlmEvent::MessageEnd { stop_reason: Some(StopReason::EndTurn) },
        ];

        let s = collect_summary(&events);
        assert_eq!(s.thinking, "Let me think about this.");
        assert_eq!(s.text, "Here's my answer.");
    }

    // ── collect_summary: interleaved content blocks ─────────────────

    #[test]
    fn summary_interleaved_blocks() {
        let events = vec![
            LlmEvent::MessageStart { message_id: None, model: None },
            LlmEvent::ThinkingDelta { index: 0, text: "think".into() },
            LlmEvent::ContentBlockEnd { index: 0 },
            LlmEvent::TextDelta { index: 1, text: "text".into() },
            LlmEvent::ContentBlockEnd { index: 1 },
            LlmEvent::ToolCallStart { index: 2, call_id: "c1".into(), name: "fn1".into() },
            LlmEvent::ToolCallArgumentDelta { index: 2, delta: "{}".into() },
            LlmEvent::ContentBlockEnd { index: 2 },
            LlmEvent::ToolCallStart { index: 3, call_id: "c2".into(), name: "fn2".into() },
            LlmEvent::ToolCallArgumentDelta { index: 3, delta: "{\"a\":1}".into() },
            LlmEvent::ToolCallEnd { index: 3 },
            LlmEvent::MessageEnd { stop_reason: Some(StopReason::ToolUse) },
        ];

        let s = collect_summary(&events);
        assert_eq!(s.thinking, "think");
        assert_eq!(s.text, "text");
        assert_eq!(s.tool_calls.len(), 2);
        assert_eq!(s.tool_calls[0].call_id, "c1");
        assert_eq!(s.tool_calls[0].arguments, "{}");
        assert_eq!(s.tool_calls[1].call_id, "c2");
        assert_eq!(s.tool_calls[1].arguments, "{\"a\":1}");
    }

    // ── collect_summary: empty stream ───────────────────────────────

    #[test]
    fn summary_empty_events() {
        let s = collect_summary(&[]);
        assert!(s.message_id.is_none());
        assert!(s.model.is_none());
        assert!(s.text.is_empty());
        assert!(s.thinking.is_empty());
        assert!(s.tool_calls.is_empty());
        assert!(s.input_tokens.is_none());
        assert!(s.output_tokens.is_none());
        assert!(s.usage_details.is_empty());
        assert!(s.stop_reason.is_none());
    }

    // ── collect_summary: usage updates accumulate ───────────────────

    #[test]
    fn summary_multiple_usage_events() {
        let events = vec![
            LlmEvent::Usage { input_tokens: Some(10), output_tokens: Some(1), details: BTreeMap::new() },
            LlmEvent::TextDelta { index: 0, text: "hi".into() },
            LlmEvent::Usage { input_tokens: None, output_tokens: Some(5), details: BTreeMap::new() },
        ];

        let s = collect_summary(&events);
        // Last wins for each field
        assert_eq!(s.input_tokens, Some(10));
        assert_eq!(s.output_tokens, Some(5));
    }

    // ── collect_summary: tool calls without explicit end ────────────

    #[test]
    fn summary_tool_call_without_end() {
        let events = vec![
            LlmEvent::ToolCallStart { index: 0, call_id: "c1".into(), name: "fn".into() },
            LlmEvent::ToolCallArgumentDelta { index: 0, delta: "{\"x\":1}".into() },
            LlmEvent::MessageEnd { stop_reason: Some(StopReason::ToolUse) },
        ];

        let s = collect_summary(&events);
        // Tool call should still be captured even without explicit end
        assert_eq!(s.tool_calls.len(), 1);
        assert_eq!(s.tool_calls[0].arguments, "{\"x\":1}");
    }

    // ── collect_summary: unknown events ignored ─────────────────────

    #[test]
    fn summary_unknown_events_ignored() {
        let events = vec![
            LlmEvent::Unknown { event_type: Some("ping".into()), raw: "".into() },
            LlmEvent::TextDelta { index: 0, text: "hello".into() },
            LlmEvent::Unknown { event_type: None, raw: "garbage".into() },
            LlmEvent::MessageEnd { stop_reason: Some(StopReason::EndTurn) },
        ];

        let s = collect_summary(&events);
        assert_eq!(s.text, "hello");
        assert_eq!(s.stop_reason, Some(StopReason::EndTurn));
    }

    // ── collect_summary: usage_details propagated ────────────────────

    #[test]
    fn summary_usage_details() {
        let events = vec![
            LlmEvent::Usage {
                input_tokens: Some(100),
                output_tokens: Some(50),
                details: BTreeMap::from([("cache_read".into(), 80)]),
            },
            LlmEvent::TextDelta { index: 0, text: "cached".into() },
            LlmEvent::Usage {
                input_tokens: None,
                output_tokens: Some(60),
                details: BTreeMap::from([("thinking".into(), 20)]),
            },
        ];

        let s = collect_summary(&events);
        assert_eq!(s.input_tokens, Some(100));
        assert_eq!(s.output_tokens, Some(60));
        // Both keys should be present (merge)
        assert_eq!(s.usage_details.get("cache_read"), Some(&80));
        assert_eq!(s.usage_details.get("thinking"), Some(&20));
    }

    // ── collect_summary: sorted tool calls ──────────────────────────

    #[test]
    fn summary_tool_calls_sorted_by_index() {
        let events = vec![
            LlmEvent::ToolCallStart { index: 2, call_id: "c2".into(), name: "b".into() },
            LlmEvent::ToolCallEnd { index: 2 },
            LlmEvent::ToolCallStart { index: 0, call_id: "c0".into(), name: "a".into() },
            LlmEvent::ToolCallEnd { index: 0 },
        ];

        let s = collect_summary(&events);
        assert_eq!(s.tool_calls[0].index, 0);
        assert_eq!(s.tool_calls[1].index, 2);
    }

    // ── parse_non_streaming_usage ────────────────────────────────────

    use super::super::provider::ProviderKind;

    #[test]
    fn non_streaming_google_usage() {
        let body = br#"{
            "modelVersion": "gemini-2.5-flash-preview-05-20",
            "usageMetadata": {
                "promptTokenCount": 100,
                "candidatesTokenCount": 50,
                "thoughtsTokenCount": 20
            }
        }"#;
        let (model, input, output, details) = parse_non_streaming_usage(ProviderKind::Google, body);
        assert_eq!(model.as_deref(), Some("gemini-2.5-flash-preview-05-20"));
        assert_eq!(input, Some(100));
        assert_eq!(output, Some(50));
        assert_eq!(details.get("thinking"), Some(&20));
    }

    #[test]
    fn non_streaming_anthropic_usage() {
        let body = br#"{
            "model": "claude-sonnet-4-20250514",
            "usage": {
                "input_tokens": 200,
                "output_tokens": 80,
                "cache_read_input_tokens": 150
            }
        }"#;
        let (model, input, output, details) = parse_non_streaming_usage(ProviderKind::Anthropic, body);
        assert_eq!(model.as_deref(), Some("claude-sonnet-4-20250514"));
        assert_eq!(input, Some(200));
        assert_eq!(output, Some(80));
        assert_eq!(details.get("cache_read"), Some(&150));
    }

    #[test]
    fn non_streaming_openai_usage() {
        let body = br#"{
            "model": "gpt-4o",
            "usage": {
                "prompt_tokens": 300,
                "completion_tokens": 120,
                "prompt_tokens_details": {"cached_tokens": 50},
                "completion_tokens_details": {"reasoning_tokens": 30}
            }
        }"#;
        let (model, input, output, details) = parse_non_streaming_usage(ProviderKind::OpenAi, body);
        assert_eq!(model.as_deref(), Some("gpt-4o"));
        assert_eq!(input, Some(300));
        assert_eq!(output, Some(120));
        assert_eq!(details.get("cache_read"), Some(&50));
        assert_eq!(details.get("thinking"), Some(&30));
    }

    #[test]
    fn non_streaming_invalid_json() {
        let (model, input, output, details) = parse_non_streaming_usage(ProviderKind::Google, b"not json");
        assert!(model.is_none());
        assert!(input.is_none());
        assert!(output.is_none());
        assert!(details.is_empty());
    }

    #[test]
    fn non_streaming_empty_body() {
        let (model, input, output, details) = parse_non_streaming_usage(ProviderKind::Anthropic, b"");
        assert!(model.is_none());
        assert!(input.is_none());
        assert!(output.is_none());
        assert!(details.is_empty());
    }

    #[test]
    fn non_streaming_gzip_compressed() {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;

        let json = br#"{
            "modelVersion": "gemini-2.5-flash-lite",
            "usageMetadata": {
                "promptTokenCount": 42,
                "candidatesTokenCount": 7
            }
        }"#;
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(json).unwrap();
        let compressed = encoder.finish().unwrap();

        let (model, input, output, _) = parse_non_streaming_usage(ProviderKind::Google, &compressed);
        assert_eq!(model.as_deref(), Some("gemini-2.5-flash-lite"));
        assert_eq!(input, Some(42));
        assert_eq!(output, Some(7));
    }

    #[test]
    fn non_streaming_corrupt_gzip() {
        // Gzip magic bytes but corrupt data
        let body = &[0x1f, 0x8b, 0x00, 0x00, 0xff, 0xff];
        let (model, input, output, details) = parse_non_streaming_usage(ProviderKind::Google, body);
        assert!(model.is_none());
        assert!(input.is_none());
        assert!(output.is_none());
        assert!(details.is_empty());
    }
}
