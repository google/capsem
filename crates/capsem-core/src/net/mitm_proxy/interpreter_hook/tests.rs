use super::super::hooks::{ChunkCtx, ChunkHook, ConnMeta, HookState};
use super::super::sse_parser_hook::{SseEventStream, SseParserHook};
use super::*;

fn ctx_for<'a>(state: &'a mut HookState, conn: &'a ConnMeta) -> ChunkCtx<'a> {
    ChunkCtx {
        state,
        conn,
        trace_id: None,
    }
}

fn anthropic_conn() -> ConnMeta {
    ConnMeta {
        domain: "api.anthropic.com".into(),
        port: 443,
        process_name: None,
        ..Default::default()
    }
}

fn openai_conn() -> ConnMeta {
    ConnMeta {
        domain: "api.openai.com".into(),
        port: 443,
        process_name: None,
        ..Default::default()
    }
}

fn local_openai_conn() -> ConnMeta {
    ConnMeta {
        domain: "127.0.0.1".into(),
        port: 11434,
        process_name: None,
        ai_provider: Some(ProviderKind::OpenAi),
        ..Default::default()
    }
}

fn google_conn() -> ConnMeta {
    ConnMeta {
        domain: "generativelanguage.googleapis.com".into(),
        port: 443,
        process_name: None,
        ..Default::default()
    }
}

/// End-to-end: SseParserHook → AnthropicInterpreterHook on the same
/// chunk. Verifies that the matching interpreter drains
/// SseEventStream and pushes LlmEvents tagged with the provider.
#[test]
fn anthropic_pipeline_produces_llm_events_with_provider_tag() {
    let sse = SseParserHook::new();
    let interp = AnthropicInterpreterHook::new();
    let mut state = HookState::default();
    let conn = anthropic_conn();

    let body = "event: message_start\n\
                data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"model\":\"claude-test\"}}\n\
                \n\
                event: content_block_start\n\
                data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\"}}\n\
                \n\
                event: content_block_delta\n\
                data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}\n\
                \n\
                event: content_block_stop\n\
                data: {\"type\":\"content_block_stop\",\"index\":0}\n\
                \n\
                event: message_stop\n\
                data: {}\n\n";
    let mut chunk = Bytes::from(body);

    {
        let mut ctx = ctx_for(&mut state, &conn);
        sse.on_response_chunk(&mut chunk, &mut ctx);
    }
    {
        let mut ctx = ctx_for(&mut state, &conn);
        interp.on_response_chunk(&mut chunk, &mut ctx);
    }

    // Interpreter drained SSE events.
    let sse_stream = state.peek::<SseEventStream>().expect("SseEventStream slot");
    assert!(
        sse_stream.events.is_empty(),
        "interpreter must drain the SSE queue"
    );

    // LlmEventStream populated with provider tag.
    let llm = state.peek::<LlmEventStream>().expect("LlmEventStream slot");
    assert_eq!(llm.provider, Some(ProviderKind::Anthropic));
    assert!(
        !llm.events.is_empty(),
        "interpreter should produce LlmEvents"
    );

    // Sanity: collect_summary works against the accumulated events.
    let summary = crate::net::ai_traffic::events::collect_summary(&llm.events);
    assert_eq!(summary.message_id.as_deref(), Some("msg_1"));
    assert_eq!(summary.model.as_deref(), Some("claude-test"));
    assert_eq!(summary.text, "hello");
}

/// Non-matching domain: hook is a no-op even with SSE events queued.
#[test]
fn anthropic_hook_skips_on_wrong_domain() {
    let interp = AnthropicInterpreterHook::new();
    let mut state = HookState::default();
    let conn = openai_conn();

    // Seed SseEventStream directly via the slot map so we can verify
    // the hook leaves it untouched on a non-matching domain.
    {
        let mut c = ChunkCtx {
            state: &mut state,
            conn: &conn,
            trace_id: None,
        };
        let s = c.state::<SseEventStream>(SseEventStream::default);
        s.events.push(crate::net::parsers::sse_parser::SseEvent {
            event_type: Some("message_start".into()),
            data: "{}".into(),
        });
    }

    {
        let mut ctx = ctx_for(&mut state, &conn);
        interp.on_response_chunk(&mut Bytes::new(), &mut ctx);
    }

    // Interpreter must not have drained the queue.
    let sse_stream = state.peek::<SseEventStream>().expect("slot");
    assert_eq!(sse_stream.events.len(), 1);
    // No LlmEventStream allocated.
    assert!(state.peek::<LlmEventStream>().is_none());
}

/// OpenAI provider routes through OpenAiInterpreterHook on its domain.
#[test]
fn openai_pipeline_produces_llm_events() {
    let sse = SseParserHook::new();
    let interp = OpenAiInterpreterHook::new();
    let mut state = HookState::default();
    let conn = openai_conn();

    let body = "data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\
                \"choices\":[{\"index\":0,\"delta\":{\"content\":\"hi\"}}]}\n\n\
                data: [DONE]\n\n";
    let mut chunk = Bytes::from(body);

    {
        let mut ctx = ctx_for(&mut state, &conn);
        sse.on_response_chunk(&mut chunk, &mut ctx);
    }
    {
        let mut ctx = ctx_for(&mut state, &conn);
        interp.on_response_chunk(&mut chunk, &mut ctx);
    }

    let llm = state.peek::<LlmEventStream>().expect("LlmEventStream slot");
    assert_eq!(llm.provider, Some(ProviderKind::OpenAi));
    assert!(
        llm.events
            .iter()
            .any(|e| matches!(e, LlmEvent::TextDelta { text, .. } if text == "hi")),
        "expected TextDelta for the OpenAI chunk"
    );
}

#[test]
fn explicit_openai_provider_routes_local_streams_to_openai_interpreter() {
    let sse = SseParserHook::new();
    let interp = OpenAiInterpreterHook::new();
    let mut state = HookState::default();
    let conn = local_openai_conn();

    let body = "data: {\"id\":\"chatcmpl-local\",\"model\":\"gpt-local\",\
                \"choices\":[{\"index\":0,\"delta\":{\"content\":\"hi\"}}]}\n\n\
                data: [DONE]\n\n";
    let mut chunk = Bytes::from(body);

    {
        let mut ctx = ctx_for(&mut state, &conn);
        sse.on_response_chunk(&mut chunk, &mut ctx);
    }
    {
        let mut ctx = ctx_for(&mut state, &conn);
        interp.on_response_chunk(&mut chunk, &mut ctx);
    }

    let llm = state.peek::<LlmEventStream>().expect("LlmEventStream slot");
    assert_eq!(llm.provider, Some(ProviderKind::OpenAi));
    assert!(
        llm.events
            .iter()
            .any(|e| matches!(e, LlmEvent::TextDelta { text, .. } if text == "hi")),
        "expected TextDelta for the local OpenAI-compatible chunk"
    );
}

/// Google provider routes through GoogleInterpreterHook on its domain.
#[test]
fn google_pipeline_produces_llm_events() {
    let sse = SseParserHook::new();
    let interp = GoogleInterpreterHook::new();
    let mut state = HookState::default();
    let conn = google_conn();

    let body = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"hi\"}]}}],\
                \"usageMetadata\":{\"promptTokenCount\":1,\"candidatesTokenCount\":2}}\n\n";
    let mut chunk = Bytes::from(body);

    {
        let mut ctx = ctx_for(&mut state, &conn);
        sse.on_response_chunk(&mut chunk, &mut ctx);
    }
    {
        let mut ctx = ctx_for(&mut state, &conn);
        interp.on_response_chunk(&mut chunk, &mut ctx);
    }

    let llm = state.peek::<LlmEventStream>().expect("LlmEventStream slot");
    assert_eq!(llm.provider, Some(ProviderKind::Google));
    assert!(
        !llm.events.is_empty(),
        "expected at least one Google LlmEvent"
    );
}

/// All three interpreter hooks coexisting in one pipeline state map:
/// only the matching one drains.
#[test]
fn three_hooks_only_matching_one_processes() {
    let sse = SseParserHook::new();
    let a = AnthropicInterpreterHook::new();
    let o = OpenAiInterpreterHook::new();
    let g = GoogleInterpreterHook::new();
    let mut state = HookState::default();
    let conn = openai_conn();

    let body = "data: {\"id\":\"x\",\"object\":\"chat.completion.chunk\",\
                \"choices\":[{\"index\":0,\"delta\":{\"content\":\"yo\"}}]}\n\n";
    let mut chunk = Bytes::from(body);

    {
        let mut ctx = ctx_for(&mut state, &conn);
        sse.on_response_chunk(&mut chunk, &mut ctx);
    }
    // All three run in registration order; only OpenAI matches.
    for hook in [
        &a as &dyn ChunkHook,
        &o as &dyn ChunkHook,
        &g as &dyn ChunkHook,
    ] {
        let mut ctx = ctx_for(&mut state, &conn);
        hook.on_response_chunk(&mut chunk, &mut ctx);
    }

    let llm = state.peek::<LlmEventStream>().expect("slot");
    assert_eq!(llm.provider, Some(ProviderKind::OpenAi));
    let sse_stream = state.peek::<SseEventStream>().expect("slot");
    assert!(sse_stream.events.is_empty());
}

/// on_response_end runs the same drain so trailing SSE events are
/// consumed.
#[test]
fn on_response_end_drains_trailing_events() {
    let sse = SseParserHook::new();
    let interp = AnthropicInterpreterHook::new();
    let mut state = HookState::default();
    let conn = anthropic_conn();

    // Push partial chunk then end without final blank line.
    let mut chunk = Bytes::from(
        "event: message_start\n\
         data: {\"type\":\"message_start\",\"message\":{\"id\":\"m1\",\"model\":\"x\"}}\n",
    );
    {
        let mut ctx = ctx_for(&mut state, &conn);
        sse.on_response_chunk(&mut chunk, &mut ctx);
    }
    {
        let mut ctx = ctx_for(&mut state, &conn);
        interp.on_response_chunk(&mut chunk, &mut ctx);
    }
    // Nothing processed yet (no blank line).
    assert!(state
        .peek::<LlmEventStream>()
        .is_none_or(|s| s.events.is_empty()));

    // End-of-stream flushes the SSE parser, then interpreter drains.
    {
        let mut ctx = ctx_for(&mut state, &conn);
        sse.on_response_end(&mut ctx);
    }
    {
        let mut ctx = ctx_for(&mut state, &conn);
        interp.on_response_end(&mut ctx);
    }
    let llm = state.peek::<LlmEventStream>().expect("slot");
    assert!(
        !llm.events.is_empty(),
        "trailing event should reach interpreter"
    );
}
