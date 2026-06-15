use super::*;
use crate::net::ai_traffic::events::collect_summary;
use crate::net::parsers::sse_parser::SseParser;

#[test]
fn upstream_url_responses() {
    let p = OpenAiProvider;
    assert_eq!(
        p.upstream_url("/v1/responses", None),
        "https://api.openai.com/v1/responses"
    );
}

#[test]
fn upstream_url_chat_completions() {
    let p = OpenAiProvider;
    assert_eq!(
        p.upstream_url("/v1/chat/completions", None),
        "https://api.openai.com/v1/chat/completions"
    );
}

#[test]
fn kind_is_openai() {
    assert_eq!(OpenAiProvider.kind(), ModelProtocol::OpenAi);
}

// ── Stream parser: text-only response ───────────────────────────

#[test]
fn stream_text_response() {
    let raw = b"\
data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"\"},\"finish_reason\":null}]}\n\
\n\
data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}\n\
\n\
data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\" there!\"},\"finish_reason\":null}]}\n\
\n\
data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":3}}\n\
\n\
data: [DONE]\n\
\n";

    let mut sse_parser = SseParser::new();
    let sse_events = sse_parser.feed(raw);

    let mut parser = OpenAiStreamParser::new();
    let mut llm_events = Vec::new();
    for sse in &sse_events {
        llm_events.extend(parser.parse_event(sse));
    }

    let summary = collect_summary(&llm_events);
    assert_eq!(summary.message_id.as_deref(), Some("chatcmpl-1"));
    assert_eq!(summary.model.as_deref(), Some("gpt-4o"));
    assert_eq!(summary.text, "Hello there!");
    assert_eq!(summary.stop_reason, Some(StopReason::EndTurn));
    assert_eq!(summary.input_tokens, Some(10));
    assert_eq!(summary.output_tokens, Some(3));
}

// ── Stream parser: tool calls ───────────────────────────────────

#[test]
fn stream_tool_call_response() {
    let raw = b"\
data: {\"id\":\"chatcmpl-2\",\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"tool_calls\":[{\"index\":0,\"id\":\"call_abc\",\"type\":\"function\",\"function\":{\"name\":\"get_weather\",\"arguments\":\"\"}}]},\"finish_reason\":null}]}\n\
\n\
data: {\"id\":\"chatcmpl-2\",\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"city\\\"\"}}]},\"finish_reason\":null}]}\n\
\n\
data: {\"id\":\"chatcmpl-2\",\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\": \\\"NYC\\\"}\"}}]},\"finish_reason\":null}]}\n\
\n\
data: {\"id\":\"chatcmpl-2\",\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\
\n\
data: [DONE]\n\
\n";

    let mut sse_parser = SseParser::new();
    let sse_events = sse_parser.feed(raw);

    let mut parser = OpenAiStreamParser::new();
    let mut llm_events = Vec::new();
    for sse in &sse_events {
        llm_events.extend(parser.parse_event(sse));
    }

    let summary = collect_summary(&llm_events);
    assert_eq!(summary.tool_calls.len(), 1);
    assert_eq!(summary.tool_calls[0].call_id, "call_abc");
    assert_eq!(summary.tool_calls[0].name, "get_weather");
    assert_eq!(summary.tool_calls[0].arguments, "{\"city\": \"NYC\"}");
    assert_eq!(summary.stop_reason, Some(StopReason::ToolUse));
}

// ── Adversarial: malformed JSON ─────────────────────────────────

#[test]
fn malformed_json_becomes_unknown() {
    let mut parser = OpenAiStreamParser::new();
    let sse = SseEvent {
        event_type: None,
        data: "not json".into(),
    };
    let events = parser.parse_event(&sse);
    assert_eq!(events.len(), 1);
    matches!(&events[0], LlmEvent::Unknown { .. });
}

// ── Adversarial: empty choices array ────────────────────────────

#[test]
fn empty_choices_just_starts() {
    let mut parser = OpenAiStreamParser::new();
    let sse = SseEvent {
        event_type: None,
        data: "{\"id\":\"x\",\"choices\":[]}".into(),
    };
    let events = parser.parse_event(&sse);
    // Should emit MessageStart only
    assert_eq!(events.len(), 1);
    matches!(&events[0], LlmEvent::MessageStart { .. });
}

// ── Adversarial: content_filter finish reason ───────────────────

#[test]
fn content_filter_stop_reason() {
    let mut parser = OpenAiStreamParser::new();
    let sse = SseEvent {
        event_type: None,
        data: "{\"id\":\"x\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"content_filter\"}]}".into(),
    };
    let events = parser.parse_event(&sse);
    let has_end = events.iter().any(|e| {
        matches!(
            e,
            LlmEvent::MessageEnd {
                stop_reason: Some(StopReason::ContentFilter)
            }
        )
    });
    assert!(has_end);
}

// ── Responses API: text-only response ─────────────────────────

#[test]
fn responses_api_text_response() {
    let raw = b"\
event: response.created\n\
data: {\"response\":{\"id\":\"resp-1\",\"model\":\"gpt-4o\"}}\n\
\n\
event: response.output_text.delta\n\
data: {\"output_index\":0,\"content_index\":0,\"delta\":\"Hello\"}\n\
\n\
event: response.output_text.delta\n\
data: {\"output_index\":0,\"content_index\":0,\"delta\":\" world!\"}\n\
\n\
event: response.completed\n\
data: {\"response\":{\"id\":\"resp-1\",\"model\":\"gpt-4o\",\"usage\":{\"input_tokens\":15,\"output_tokens\":5}}}\n\
\n";

    let mut sse_parser = SseParser::new();
    let sse_events = sse_parser.feed(raw);

    let mut parser = OpenAiStreamParser::new();
    let mut llm_events = Vec::new();
    for sse in &sse_events {
        llm_events.extend(parser.parse_event(sse));
    }

    let summary = collect_summary(&llm_events);
    assert_eq!(summary.message_id.as_deref(), Some("resp-1"));
    assert_eq!(summary.model.as_deref(), Some("gpt-4o"));
    assert_eq!(summary.text, "Hello world!");
    assert_eq!(summary.stop_reason, Some(StopReason::EndTurn));
    assert_eq!(summary.input_tokens, Some(15));
    assert_eq!(summary.output_tokens, Some(5));
}

// ── Responses API: tool calls ─────────────────────────────────

#[test]
fn responses_api_tool_call() {
    let raw = b"\
event: response.created\n\
data: {\"response\":{\"id\":\"resp-2\",\"model\":\"gpt-4o\"}}\n\
\n\
event: response.output_item.added\n\
data: {\"output_index\":0,\"item\":{\"id\":\"fc_1\",\"type\":\"function_call\",\"call_id\":\"call_xyz\",\"name\":\"get_weather\"}}\n\
\n\
event: response.function_call_arguments.delta\n\
data: {\"output_index\":0,\"item_id\":\"fc_1\",\"delta\":\"{\\\"city\\\"\"}\n\
\n\
event: response.function_call_arguments.delta\n\
data: {\"output_index\":0,\"item_id\":\"fc_1\",\"delta\":\": \\\"NYC\\\"}\"}\n\
\n\
event: response.output_item.done\n\
data: {\"output_index\":0,\"item\":{\"id\":\"fc_1\",\"type\":\"function_call\"}}\n\
\n\
event: response.completed\n\
data: {\"response\":{\"id\":\"resp-2\",\"model\":\"gpt-4o\",\"usage\":{\"input_tokens\":20,\"output_tokens\":10}}}\n\
\n";

    let mut sse_parser = SseParser::new();
    let sse_events = sse_parser.feed(raw);

    let mut parser = OpenAiStreamParser::new();
    let mut llm_events = Vec::new();
    for sse in &sse_events {
        llm_events.extend(parser.parse_event(sse));
    }

    let summary = collect_summary(&llm_events);
    assert_eq!(summary.tool_calls.len(), 1);
    assert_eq!(summary.tool_calls[0].call_id, "call_xyz");
    assert_eq!(summary.tool_calls[0].name, "get_weather");
    assert_eq!(summary.tool_calls[0].arguments, "{\"city\": \"NYC\"}");
    assert_eq!(summary.stop_reason, Some(StopReason::EndTurn));
    assert_eq!(summary.input_tokens, Some(20));
}

// ── Responses API: reasoning summary ──────────────────────────

#[test]
fn responses_api_reasoning_summary() {
    let raw = b"\
event: response.created\n\
data: {\"response\":{\"id\":\"resp-3\",\"model\":\"o3\"}}\n\
\n\
event: response.reasoning_summary_text.delta\n\
data: {\"output_index\":0,\"summary_index\":0,\"delta\":\"Let me think\"}\n\
\n\
event: response.reasoning_summary_text.delta\n\
data: {\"output_index\":0,\"summary_index\":0,\"delta\":\" about this.\"}\n\
\n\
event: response.output_text.delta\n\
data: {\"output_index\":1,\"content_index\":0,\"delta\":\"The answer is 42.\"}\n\
\n\
event: response.completed\n\
data: {\"response\":{\"id\":\"resp-3\",\"model\":\"o3\",\"usage\":{\"input_tokens\":10,\"output_tokens\":20,\"output_tokens_details\":{\"reasoning_tokens\":50}}}}\n\
\n";

    let mut sse_parser = SseParser::new();
    let sse_events = sse_parser.feed(raw);

    let mut parser = OpenAiStreamParser::new();
    let mut llm_events = Vec::new();
    for sse in &sse_events {
        llm_events.extend(parser.parse_event(sse));
    }

    let summary = collect_summary(&llm_events);
    assert_eq!(summary.text, "The answer is 42.");
    assert_eq!(summary.thinking, "Let me think about this.");
    assert_eq!(summary.usage_details.get("thinking"), Some(&50));
}

// ── Responses API: unknown event types are silently ignored ───

#[test]
fn responses_api_unknown_event_ignored() {
    let mut parser = OpenAiStreamParser::new();
    let sse = SseEvent {
        event_type: Some("response.in_progress".into()),
        data: "{}".into(),
    };
    let events = parser.parse_event(&sse);
    assert!(events.is_empty());
}

// ── Responses API: malformed JSON returns empty ───────────────

#[test]
fn responses_api_malformed_json() {
    let mut parser = OpenAiStreamParser::new();
    let sse = SseEvent {
        event_type: Some("response.created".into()),
        data: "not json".into(),
    };
    let events = parser.parse_event(&sse);
    assert!(events.is_empty());
}

// ── Responses API: cached + reasoning token details ──────────

#[test]
fn responses_api_usage_details() {
    let raw = b"\
event: response.created\n\
data: {\"response\":{\"id\":\"resp-4\",\"model\":\"gpt-4o\"}}\n\
\n\
event: response.completed\n\
data: {\"response\":{\"id\":\"resp-4\",\"model\":\"gpt-4o\",\"usage\":{\"input_tokens\":1000,\"output_tokens\":500,\"input_tokens_details\":{\"cached_tokens\":800},\"output_tokens_details\":{\"reasoning_tokens\":200}}}}\n\
\n";

    let mut sse_parser = SseParser::new();
    let sse_events = sse_parser.feed(raw);

    let mut parser = OpenAiStreamParser::new();
    let mut llm_events = Vec::new();
    for sse in &sse_events {
        llm_events.extend(parser.parse_event(sse));
    }

    let summary = collect_summary(&llm_events);
    assert_eq!(summary.input_tokens, Some(1000));
    assert_eq!(summary.output_tokens, Some(500));
    assert_eq!(summary.usage_details.get("cache_read"), Some(&800));
    assert_eq!(summary.usage_details.get("thinking"), Some(&200));
}

// ── Responses API: output_item.done only emits ToolCallEnd for function_call ──

#[test]
fn responses_api_output_item_done_text_ignored() {
    let mut parser = OpenAiStreamParser::new();
    // response.created to start
    let sse1 = SseEvent {
        event_type: Some("response.created".into()),
        data: r#"{"response":{"id":"resp-t","model":"gpt-4o"}}"#.into(),
    };
    parser.parse_event(&sse1);

    // output_item.done for a text message item (not function_call)
    let sse = SseEvent {
        event_type: Some("response.output_item.done".into()),
        data: r#"{"output_index":0,"item":{"id":"msg_1","type":"message"}}"#.into(),
    };
    let events = parser.parse_event(&sse);
    // Should NOT emit ToolCallEnd for a message item
    assert!(
        events.is_empty(),
        "text output_item.done should not emit ToolCallEnd"
    );
}

#[test]
fn responses_api_output_item_done_function_call_emits_end() {
    let mut parser = OpenAiStreamParser::new();
    let sse = SseEvent {
        event_type: Some("response.output_item.done".into()),
        data: r#"{"output_index":1,"item":{"id":"fc_1","type":"function_call"}}"#.into(),
    };
    let events = parser.parse_event(&sse);
    assert_eq!(events.len(), 1);
    match &events[0] {
        LlmEvent::ToolCallEnd { index } => assert_eq!(*index, 1),
        other => panic!("expected ToolCallEnd, got {:?}", other),
    }
}
