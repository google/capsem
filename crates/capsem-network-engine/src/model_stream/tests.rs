use super::*;

// ── collect_summary: text-only stream ───────────────────────────

#[test]
fn summary_text_only() {
    let events = vec![
        LlmEvent::MessageStart {
            message_id: Some("msg_01".into()),
            model: Some("claude-sonnet-4-20250514".into()),
        },
        LlmEvent::TextDelta {
            index: 0,
            text: "Hello".into(),
        },
        LlmEvent::TextDelta {
            index: 0,
            text: " world".into(),
        },
        LlmEvent::Usage {
            input_tokens: Some(10),
            output_tokens: Some(5),
            details: BTreeMap::new(),
        },
        LlmEvent::MessageEnd {
            stop_reason: Some(StopReason::EndTurn),
        },
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
        LlmEvent::MessageStart {
            message_id: None,
            model: None,
        },
        LlmEvent::ToolCallStart {
            index: 0,
            call_id: "call_1".into(),
            name: "get_weather".into(),
        },
        LlmEvent::ToolCallArgumentDelta {
            index: 0,
            delta: r#"{"loc"#.into(),
        },
        LlmEvent::ToolCallArgumentDelta {
            index: 0,
            delta: r#"ation":"NYC"}"#.into(),
        },
        LlmEvent::ToolCallEnd { index: 0 },
        LlmEvent::MessageEnd {
            stop_reason: Some(StopReason::ToolUse),
        },
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
        LlmEvent::MessageStart {
            message_id: Some("msg_02".into()),
            model: None,
        },
        LlmEvent::TextDelta {
            index: 0,
            text: "Let me check ".into(),
        },
        LlmEvent::TextDelta {
            index: 0,
            text: "the weather.".into(),
        },
        LlmEvent::ContentBlockEnd { index: 0 },
        LlmEvent::ToolCallStart {
            index: 1,
            call_id: "call_x".into(),
            name: "weather".into(),
        },
        LlmEvent::ToolCallArgumentDelta {
            index: 1,
            delta: "{}".into(),
        },
        LlmEvent::ToolCallEnd { index: 1 },
        LlmEvent::Usage {
            input_tokens: Some(20),
            output_tokens: Some(15),
            details: BTreeMap::new(),
        },
        LlmEvent::MessageEnd {
            stop_reason: Some(StopReason::ToolUse),
        },
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
        LlmEvent::MessageStart {
            message_id: None,
            model: None,
        },
        LlmEvent::ThinkingDelta {
            index: 0,
            text: "Let me think".into(),
        },
        LlmEvent::ThinkingDelta {
            index: 0,
            text: " about this.".into(),
        },
        LlmEvent::ContentBlockEnd { index: 0 },
        LlmEvent::TextDelta {
            index: 1,
            text: "Here's my answer.".into(),
        },
        LlmEvent::ContentBlockEnd { index: 1 },
        LlmEvent::MessageEnd {
            stop_reason: Some(StopReason::EndTurn),
        },
    ];

    let s = collect_summary(&events);
    assert_eq!(s.thinking, "Let me think about this.");
    assert_eq!(s.text, "Here's my answer.");
}

// ── collect_summary: interleaved content blocks ─────────────────

#[test]
fn summary_interleaved_blocks() {
    let events = vec![
        LlmEvent::MessageStart {
            message_id: None,
            model: None,
        },
        LlmEvent::ThinkingDelta {
            index: 0,
            text: "think".into(),
        },
        LlmEvent::ContentBlockEnd { index: 0 },
        LlmEvent::TextDelta {
            index: 1,
            text: "text".into(),
        },
        LlmEvent::ContentBlockEnd { index: 1 },
        LlmEvent::ToolCallStart {
            index: 2,
            call_id: "c1".into(),
            name: "fn1".into(),
        },
        LlmEvent::ToolCallArgumentDelta {
            index: 2,
            delta: "{}".into(),
        },
        LlmEvent::ContentBlockEnd { index: 2 },
        LlmEvent::ToolCallStart {
            index: 3,
            call_id: "c2".into(),
            name: "fn2".into(),
        },
        LlmEvent::ToolCallArgumentDelta {
            index: 3,
            delta: "{\"a\":1}".into(),
        },
        LlmEvent::ToolCallEnd { index: 3 },
        LlmEvent::MessageEnd {
            stop_reason: Some(StopReason::ToolUse),
        },
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
        LlmEvent::Usage {
            input_tokens: Some(10),
            output_tokens: Some(1),
            details: BTreeMap::new(),
        },
        LlmEvent::TextDelta {
            index: 0,
            text: "hi".into(),
        },
        LlmEvent::Usage {
            input_tokens: None,
            output_tokens: Some(5),
            details: BTreeMap::new(),
        },
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
        LlmEvent::ToolCallStart {
            index: 0,
            call_id: "c1".into(),
            name: "fn".into(),
        },
        LlmEvent::ToolCallArgumentDelta {
            index: 0,
            delta: "{\"x\":1}".into(),
        },
        LlmEvent::MessageEnd {
            stop_reason: Some(StopReason::ToolUse),
        },
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
        LlmEvent::Unknown {
            event_type: Some("ping".into()),
            raw: "".into(),
        },
        LlmEvent::TextDelta {
            index: 0,
            text: "hello".into(),
        },
        LlmEvent::Unknown {
            event_type: None,
            raw: "garbage".into(),
        },
        LlmEvent::MessageEnd {
            stop_reason: Some(StopReason::EndTurn),
        },
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
        LlmEvent::TextDelta {
            index: 0,
            text: "cached".into(),
        },
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
        LlmEvent::ToolCallStart {
            index: 2,
            call_id: "c2".into(),
            name: "b".into(),
        },
        LlmEvent::ToolCallEnd { index: 2 },
        LlmEvent::ToolCallStart {
            index: 0,
            call_id: "c0".into(),
            name: "a".into(),
        },
        LlmEvent::ToolCallEnd { index: 0 },
    ];

    let s = collect_summary(&events);
    assert_eq!(s.tool_calls[0].index, 0);
    assert_eq!(s.tool_calls[1].index, 2);
}

// ── parse_non_streaming_usage ────────────────────────────────────

use crate::ai_provider::ProviderKind;

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
    let (model, input, output, details) =
        parse_non_streaming_usage(ProviderKind::Google, b"not json");
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
