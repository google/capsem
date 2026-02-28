/// Provider-agnostic LLM event types emitted by SSE stream parsers.
///
/// Each AI provider (Anthropic, OpenAI, Google) has its own SSE wire format.
/// Provider-specific parsers convert those into these unified events, which
/// are then collected into a `StreamSummary` for audit logging.

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
            LlmEvent::Usage { input_tokens: it, output_tokens: ot } => {
                if let Some(t) = it {
                    input_tokens = Some(*t);
                }
                if let Some(t) = ot {
                    output_tokens = Some(*t);
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
        stop_reason,
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
            LlmEvent::Usage { input_tokens: Some(10), output_tokens: Some(5) },
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
            LlmEvent::Usage { input_tokens: Some(20), output_tokens: Some(15) },
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
        assert!(s.stop_reason.is_none());
    }

    // ── collect_summary: usage updates accumulate ───────────────────

    #[test]
    fn summary_multiple_usage_events() {
        let events = vec![
            LlmEvent::Usage { input_tokens: Some(10), output_tokens: Some(1) },
            LlmEvent::TextDelta { index: 0, text: "hi".into() },
            LlmEvent::Usage { input_tokens: None, output_tokens: Some(5) },
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
}
