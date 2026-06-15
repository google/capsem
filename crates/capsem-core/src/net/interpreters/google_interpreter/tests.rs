use super::*;
use crate::net::ai_traffic::events::collect_summary;
use crate::net::parsers::sse_parser::SseParser;

#[test]
fn upstream_url_stream_generate() {
    let p = GoogleProvider;
    assert_eq!(
        p.upstream_url(
            "/v1beta/models/gemini-2.5-pro:streamGenerateContent",
            None
        ),
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-pro:streamGenerateContent"
    );
}

#[test]
fn upstream_url_generate_content() {
    let p = GoogleProvider;
    assert_eq!(
        p.upstream_url("/v1beta/models/gemini-2.5-flash:generateContent", None),
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent"
    );
}

#[test]
fn upstream_url_with_existing_query() {
    let p = GoogleProvider;
    assert_eq!(
        p.upstream_url(
            "/v1beta/models/gemini-2.5-pro:streamGenerateContent",
            Some("alt=sse")
        ),
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-pro:streamGenerateContent?alt=sse"
    );
}

#[test]
fn kind_is_google() {
    assert_eq!(GoogleProvider.kind(), ModelProtocol::Google);
}

// ── Stream parser: text response ────────────────────────────────

#[test]
fn stream_text_response() {
    let raw = b"\
data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hello\"}],\"role\":\"model\"}}],\"usageMetadata\":{\"promptTokenCount\":5,\"candidatesTokenCount\":1},\"modelVersion\":\"gemini-2.5-flash\"}\n\
\n\
data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\" world!\"}],\"role\":\"model\"}}],\"usageMetadata\":{\"promptTokenCount\":5,\"candidatesTokenCount\":3}}\n\
\n\
data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"\"}],\"role\":\"model\"},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":5,\"candidatesTokenCount\":3,\"totalTokenCount\":8}}\n\
\n";

    let mut sse_parser = SseParser::new();
    let sse_events = sse_parser.feed(raw);

    let mut parser = GoogleStreamParser::new();
    let mut llm_events = Vec::new();
    for sse in &sse_events {
        llm_events.extend(parser.parse_event(sse));
    }

    let summary = collect_summary(&llm_events);
    assert_eq!(summary.model.as_deref(), Some("gemini-2.5-flash"));
    assert_eq!(summary.text, "Hello world!");
    assert_eq!(summary.stop_reason, Some(StopReason::EndTurn));
    assert_eq!(summary.input_tokens, Some(5));
    assert_eq!(summary.output_tokens, Some(3));
}

// ── Stream parser: function call ────────────────────────────────

#[test]
fn stream_function_call_response() {
    let raw = b"\
data: {\"candidates\":[{\"content\":{\"parts\":[{\"functionCall\":{\"name\":\"get_weather\",\"args\":{\"city\":\"NYC\"}}}],\"role\":\"model\"},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":10,\"candidatesTokenCount\":5}}\n\
\n";

    let mut sse_parser = SseParser::new();
    let sse_events = sse_parser.feed(raw);

    let mut parser = GoogleStreamParser::new();
    let mut llm_events = Vec::new();
    for sse in &sse_events {
        llm_events.extend(parser.parse_event(sse));
    }

    let summary = collect_summary(&llm_events);
    assert_eq!(summary.tool_calls.len(), 1);
    assert_eq!(summary.tool_calls[0].name, "get_weather");
    assert_eq!(summary.tool_calls[0].call_id, "gemini_get_weather_0");
    let args: serde_json::Value = serde_json::from_str(&summary.tool_calls[0].arguments).unwrap();
    assert_eq!(args["city"], "NYC");
}

#[test]
fn stream_function_call_preserves_arg_bytes_verbatim() {
    // args is stored as RawValue, so the emitted arguments string is the
    // exact byte slice from the wire -- whitespace, key order, and all.
    // A serde_json::Value would have re-serialized to canonical compact form.
    let raw = b"\
data: {\"candidates\":[{\"content\":{\"parts\":[{\"functionCall\":{\"name\":\"get_weather\",\"args\":{\"city\" : \"NYC\" , \"units\":\"imperial\"}}}],\"role\":\"model\"},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":10,\"candidatesTokenCount\":5}}\n\
\n";

    let mut sse_parser = SseParser::new();
    let sse_events = sse_parser.feed(raw);
    let mut parser = GoogleStreamParser::new();
    let mut llm_events = Vec::new();
    for sse in &sse_events {
        llm_events.extend(parser.parse_event(sse));
    }

    let summary = collect_summary(&llm_events);
    assert_eq!(summary.tool_calls.len(), 1);
    assert_eq!(
        summary.tool_calls[0].arguments,
        r#"{"city" : "NYC" , "units":"imperial"}"#
    );
}

// ── Stream parser: thinking ─────────────────────────────────────

#[test]
fn stream_thinking_response() {
    let raw = b"\
data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Let me reason.\",\"thought\":true}],\"role\":\"model\"}}]}\n\
\n\
data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"The answer.\"}],\"role\":\"model\"},\"finishReason\":\"STOP\"}]}\n\
\n";

    let mut sse_parser = SseParser::new();
    let sse_events = sse_parser.feed(raw);

    let mut parser = GoogleStreamParser::new();
    let mut llm_events = Vec::new();
    for sse in &sse_events {
        llm_events.extend(parser.parse_event(sse));
    }

    let summary = collect_summary(&llm_events);
    assert_eq!(summary.thinking, "Let me reason.");
    assert_eq!(summary.text, "The answer.");
}

// ── Stream parser: cache read tokens ────────────────────────────

#[test]
fn stream_cache_read_tokens() {
    let raw = b"\
data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Cached reply\"}],\"role\":\"model\"},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":100,\"candidatesTokenCount\":20,\"cachedContentTokenCount\":80},\"modelVersion\":\"gemini-2.5-pro\"}\n\
\n";

    let mut sse_parser = SseParser::new();
    let sse_events = sse_parser.feed(raw);

    let mut parser = GoogleStreamParser::new();
    let mut llm_events = Vec::new();
    for sse in &sse_events {
        llm_events.extend(parser.parse_event(sse));
    }

    let summary = collect_summary(&llm_events);
    assert_eq!(summary.input_tokens, Some(100));
    assert_eq!(summary.output_tokens, Some(20));
    assert_eq!(summary.usage_details.get("cache_read"), Some(&80));
}

// ── Stream parser: thinking tokens in usage ─────────────────────

#[test]
fn stream_thinking_tokens_in_usage() {
    let raw = b"\
data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Let me think.\",\"thought\":true}],\"role\":\"model\"}}],\"usageMetadata\":{\"promptTokenCount\":50,\"candidatesTokenCount\":10,\"thoughtsTokenCount\":200},\"modelVersion\":\"gemini-2.5-pro\"}\n\
\n\
data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Answer.\"}],\"role\":\"model\"},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":50,\"candidatesTokenCount\":15,\"thoughtsTokenCount\":200}}\n\
\n";

    let mut sse_parser = SseParser::new();
    let sse_events = sse_parser.feed(raw);

    let mut parser = GoogleStreamParser::new();
    let mut llm_events = Vec::new();
    for sse in &sse_events {
        llm_events.extend(parser.parse_event(sse));
    }

    let summary = collect_summary(&llm_events);
    assert_eq!(summary.thinking, "Let me think.");
    assert_eq!(summary.text, "Answer.");
    assert_eq!(summary.usage_details.get("thinking"), Some(&200));
}

// ── Adversarial: malformed JSON ─────────────────────────────────

#[test]
fn malformed_json_becomes_unknown() {
    let mut parser = GoogleStreamParser::new();
    let sse = SseEvent {
        event_type: None,
        data: "garbage".into(),
    };
    let events = parser.parse_event(&sse);
    assert_eq!(events.len(), 1);
    matches!(&events[0], LlmEvent::Unknown { .. });
}

// ── Adversarial: empty candidates ───────────────────────────────

#[test]
fn empty_candidates() {
    let mut parser = GoogleStreamParser::new();
    let sse = SseEvent {
        event_type: None,
        data: "{\"candidates\":[]}".into(),
    };
    let events = parser.parse_event(&sse);
    // Should emit MessageStart only
    assert_eq!(events.len(), 1);
    matches!(&events[0], LlmEvent::MessageStart { .. });
}
