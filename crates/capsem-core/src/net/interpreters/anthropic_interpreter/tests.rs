use super::*;
use crate::net::ai_traffic::events::collect_summary;
use capsem_network_engine::sse_parser::SseParser;

#[test]
fn upstream_url_messages() {
    let p = AnthropicProvider;
    assert_eq!(
        p.upstream_url("/v1/messages", None),
        "https://api.anthropic.com/v1/messages"
    );
}

#[test]
fn upstream_url_with_query() {
    let p = AnthropicProvider;
    assert_eq!(
        p.upstream_url("/v1/messages", Some("beta=true")),
        "https://api.anthropic.com/v1/messages?beta=true"
    );
}

#[test]
fn kind_is_anthropic() {
    assert_eq!(AnthropicProvider.kind(), ProviderKind::Anthropic);
}

// ── Stream parser: text-only response ───────────────────────────

#[test]
fn stream_text_response() {
    let raw = b"\
event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_01\",\"model\":\"claude-sonnet-4-20250514\",\"usage\":{\"input_tokens\":25,\"output_tokens\":1}}}\n\
\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\" world!\"}}\n\
\n\
event: content_block_stop\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\
\n\
event: message_delta\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":5}}\n\
\n\
event: message_stop\n\
data: {\"type\":\"message_stop\"}\n\
\n";

    let mut sse_parser = SseParser::new();
    let sse_events = sse_parser.feed(raw);

    let mut parser = AnthropicStreamParserWithState::new();
    let mut llm_events = Vec::new();
    for sse in &sse_events {
        llm_events.extend(parser.parse_event(sse));
    }

    let summary = collect_summary(&llm_events);
    assert_eq!(summary.message_id.as_deref(), Some("msg_01"));
    assert_eq!(summary.model.as_deref(), Some("claude-sonnet-4-20250514"));
    assert_eq!(summary.text, "Hello world!");
    assert!(summary.tool_calls.is_empty());
    assert_eq!(summary.input_tokens, Some(25));
    assert_eq!(summary.output_tokens, Some(5));
    assert_eq!(summary.stop_reason, Some(StopReason::EndTurn));
}

// ── Stream parser: tool use ─────────────────────────────────────

#[test]
fn stream_tool_use_response() {
    let raw = b"\
event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_02\",\"model\":\"claude-sonnet-4-20250514\",\"usage\":{\"input_tokens\":100,\"output_tokens\":1}}}\n\
\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"I'll check.\"}}\n\
\n\
event: content_block_stop\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\
\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_01\",\"name\":\"get_weather\",\"input\":{}}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"city\\\"\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\": \\\"NYC\\\"}\"}}\n\
\n\
event: content_block_stop\n\
data: {\"type\":\"content_block_stop\",\"index\":1}\n\
\n\
event: message_delta\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":50}}\n\
\n\
event: message_stop\n\
data: {\"type\":\"message_stop\"}\n\
\n";

    let mut sse_parser = SseParser::new();
    let sse_events = sse_parser.feed(raw);

    let mut parser = AnthropicStreamParserWithState::new();
    let mut llm_events = Vec::new();
    for sse in &sse_events {
        llm_events.extend(parser.parse_event(sse));
    }

    let summary = collect_summary(&llm_events);
    assert_eq!(summary.text, "I'll check.");
    assert_eq!(summary.tool_calls.len(), 1);
    assert_eq!(summary.tool_calls[0].call_id, "toolu_01");
    assert_eq!(summary.tool_calls[0].name, "get_weather");
    assert_eq!(summary.tool_calls[0].arguments, "{\"city\": \"NYC\"}");
    assert_eq!(summary.stop_reason, Some(StopReason::ToolUse));
}

// ── Stream parser: thinking ─────────────────────────────────────

#[test]
fn stream_thinking_response() {
    let raw = b"\
event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_03\",\"model\":\"claude-sonnet-4-20250514\"}}\n\
\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"Let me reason.\"}}\n\
\n\
event: content_block_stop\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\
\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"The answer.\"}}\n\
\n\
event: content_block_stop\n\
data: {\"type\":\"content_block_stop\",\"index\":1}\n\
\n\
event: message_delta\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":20}}\n\
\n\
event: message_stop\n\
data: {\"type\":\"message_stop\"}\n\
\n";

    let mut sse_parser = SseParser::new();
    let sse_events = sse_parser.feed(raw);

    let mut parser = AnthropicStreamParserWithState::new();
    let mut llm_events = Vec::new();
    for sse in &sse_events {
        llm_events.extend(parser.parse_event(sse));
    }

    let summary = collect_summary(&llm_events);
    assert_eq!(summary.thinking, "Let me reason.");
    assert_eq!(summary.text, "The answer.");
}

// ── Stream parser: cache_read_input_tokens ──────────────────────

#[test]
fn stream_cache_read_tokens() {
    let raw = b"\
event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_cache\",\"model\":\"claude-sonnet-4-20250514\",\"usage\":{\"input_tokens\":500,\"output_tokens\":1,\"cache_read_input_tokens\":400}}}\n\
\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Cached!\"}}\n\
\n\
event: content_block_stop\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\
\n\
event: message_delta\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":10}}\n\
\n\
event: message_stop\n\
data: {\"type\":\"message_stop\"}\n\
\n";

    let mut sse_parser = SseParser::new();
    let sse_events = sse_parser.feed(raw);

    let mut parser = AnthropicStreamParserWithState::new();
    let mut llm_events = Vec::new();
    for sse in &sse_events {
        llm_events.extend(parser.parse_event(sse));
    }

    let summary = collect_summary(&llm_events);
    assert_eq!(summary.input_tokens, Some(500));
    assert_eq!(summary.usage_details.get("cache_read"), Some(&400));
    assert_eq!(summary.text, "Cached!");
}

// ── Adversarial: malformed JSON in SSE data ─────────────────────

#[test]
fn malformed_json_becomes_unknown() {
    let mut parser = AnthropicStreamParserWithState::new();
    let sse = SseEvent {
        event_type: Some("content_block_delta".into()),
        data: "not valid json{{{".into(),
    };
    let events = parser.parse_event(&sse);
    assert_eq!(events.len(), 1);
    match &events[0] {
        LlmEvent::Unknown { event_type, raw } => {
            assert_eq!(event_type.as_deref(), Some("content_block_delta"));
            assert_eq!(raw, "not valid json{{{");
        }
        other => panic!("expected Unknown, got {:?}", other),
    }
}

// ── Adversarial: unknown event type ─────────────────────────────

#[test]
fn unknown_event_type_passthrough() {
    let mut parser = AnthropicStreamParserWithState::new();
    let sse = SseEvent {
        event_type: Some("future_event".into()),
        data: "{}".into(),
    };
    let events = parser.parse_event(&sse);
    assert_eq!(events.len(), 1);
    matches!(&events[0], LlmEvent::Unknown { .. });
}

// ── Adversarial: missing fields in JSON ─────────────────────────

#[test]
fn missing_fields_handled_gracefully() {
    let mut parser = AnthropicStreamParserWithState::new();
    // content_block_start with no content_block field
    let sse = SseEvent {
        event_type: Some("content_block_start".into()),
        data: "{\"type\":\"content_block_start\",\"index\":0}".into(),
    };
    let events = parser.parse_event(&sse);
    assert!(events.is_empty()); // No content_block -> no events
}

// ── Ping events ignored ─────────────────────────────────────────

#[test]
fn ping_events_ignored() {
    let mut parser = AnthropicStreamParserWithState::new();
    let sse = SseEvent {
        event_type: Some("ping".into()),
        data: "{}".into(),
    };
    let events = parser.parse_event(&sse);
    assert!(events.is_empty());
}
