use super::super::hooks::{ChunkCtx, ChunkHook, ConnMeta, HookState};
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
        ai_provider: Some(crate::net::ai_traffic::provider::ProviderKind::Anthropic),
        ai_protocol: Some(crate::net::ai_traffic::provider::ModelProtocol::Anthropic),
        ..Default::default()
    }
}

fn other_conn() -> ConnMeta {
    ConnMeta {
        domain: "example.com".into(),
        port: 443,
        process_name: None,
        ..Default::default()
    }
}

/// One complete event in a single chunk: parser emits it, hook stashes it.
#[test]
fn single_event_in_one_chunk_is_emitted() {
    let hook = SseParserHook::new();
    let mut state = HookState::default();
    let conn = anthropic_conn();

    let mut chunk = Bytes::from("event: message_start\ndata: {\"hello\":\"world\"}\n\n");
    {
        let mut ctx = ctx_for(&mut state, &conn);
        hook.on_response_chunk(&mut chunk, &mut ctx);
    }

    let stream = state
        .peek::<SseEventStream>()
        .expect("stream slot must exist");
    assert_eq!(stream.events.len(), 1);
    let ev = &stream.events[0];
    assert_eq!(ev.event_type.as_deref(), Some("message_start"));
    assert_eq!(ev.data, "{\"hello\":\"world\"}");
}

/// Event split across two chunks: parser keeps state, hook concatenates correctly.
#[test]
fn event_split_across_chunks_reassembles() {
    let hook = SseParserHook::new();
    let mut state = HookState::default();
    let conn = anthropic_conn();

    let mut a = Bytes::from("event: message_start\ndata: {\"hel");
    let mut b = Bytes::from("lo\":\"world\"}\n\n");
    {
        let mut ctx = ctx_for(&mut state, &conn);
        hook.on_response_chunk(&mut a, &mut ctx);
    }
    {
        let mut ctx = ctx_for(&mut state, &conn);
        hook.on_response_chunk(&mut b, &mut ctx);
    }

    let stream = state
        .peek::<SseEventStream>()
        .expect("stream slot must exist");
    assert_eq!(stream.events.len(), 1);
    assert_eq!(stream.events[0].data, "{\"hello\":\"world\"}");
}

/// Multiple events accumulate across chunks for a downstream consumer.
#[test]
fn multiple_events_accumulate_for_consumer() {
    let hook = SseParserHook::new();
    let mut state = HookState::default();
    let conn = anthropic_conn();

    let mut chunk = Bytes::from("event: a\ndata: 1\n\nevent: b\ndata: 2\n\nevent: c\ndata: 3\n\n");
    {
        let mut ctx = ctx_for(&mut state, &conn);
        hook.on_response_chunk(&mut chunk, &mut ctx);
    }

    let stream = state
        .peek::<SseEventStream>()
        .expect("stream slot must exist");
    assert_eq!(stream.events.len(), 3);
    let kinds: Vec<&str> = stream
        .events
        .iter()
        .map(|e| e.event_type.as_deref().unwrap_or(""))
        .collect();
    assert_eq!(kinds, vec!["a", "b", "c"]);
}

/// Connections without runtime model metadata bypass the parser entirely.
#[test]
fn non_ai_domain_is_skipped() {
    let hook = SseParserHook::new();
    let mut state = HookState::default();
    let conn = other_conn();

    let mut chunk = Bytes::from("event: message_start\ndata: hi\n\n");
    {
        let mut ctx = ctx_for(&mut state, &conn);
        hook.on_response_chunk(&mut chunk, &mut ctx);
    }

    assert!(state.peek::<SseEventStream>().is_none());
}

#[test]
fn cloud_domain_without_runtime_provider_metadata_is_skipped() {
    let hook = SseParserHook::new();
    let mut state = HookState::default();
    let conn = ConnMeta {
        domain: "api.openai.com".into(),
        port: 443,
        process_name: None,
        ..Default::default()
    };

    let mut chunk = Bytes::from("data: hello\n\n");
    {
        let mut ctx = ctx_for(&mut state, &conn);
        hook.on_response_chunk(&mut chunk, &mut ctx);
    }

    assert!(state.peek::<SseEventStream>().is_none());
}

/// Trailing event without a terminating blank line gets flushed by on_response_end.
#[test]
fn on_response_end_flushes_trailing_event() {
    let hook = SseParserHook::new();
    let mut state = HookState::default();
    let conn = anthropic_conn();

    let mut chunk = Bytes::from("event: trailing\ndata: last\n");
    {
        let mut ctx = ctx_for(&mut state, &conn);
        hook.on_response_chunk(&mut chunk, &mut ctx);
    }
    // Pre-end: nothing emitted because there's no blank line yet.
    assert!(state
        .peek::<SseEventStream>()
        .is_none_or(|s| s.events.is_empty()));

    {
        let mut ctx = ctx_for(&mut state, &conn);
        hook.on_response_end(&mut ctx);
    }
    let stream = state
        .peek::<SseEventStream>()
        .expect("stream slot must exist");
    assert_eq!(stream.events.len(), 1);
    assert_eq!(stream.events[0].data, "last");
}

/// `[DONE]` sentinel from OpenAI is filtered by the parser.
#[test]
fn openai_done_sentinel_is_filtered() {
    let hook = SseParserHook::new();
    let mut state = HookState::default();
    let conn = ConnMeta {
        domain: "api.openai.com".into(),
        port: 443,
        process_name: None,
        ai_provider: Some(crate::net::ai_traffic::provider::ProviderKind::OpenAi),
        ai_protocol: Some(crate::net::ai_traffic::provider::ModelProtocol::OpenAi),
        ..Default::default()
    };

    let mut chunk = Bytes::from("data: hello\n\ndata: [DONE]\n\n");
    {
        let mut ctx = ctx_for(&mut state, &conn);
        hook.on_response_chunk(&mut chunk, &mut ctx);
    }

    let stream = state
        .peek::<SseEventStream>()
        .expect("stream slot must exist");
    assert_eq!(stream.events.len(), 1);
    assert_eq!(stream.events[0].data, "hello");
}

#[test]
fn explicit_ai_provider_enables_local_openai_compatible_streams() {
    let hook = SseParserHook::new();
    let mut state = HookState::default();
    let conn = ConnMeta {
        domain: "127.0.0.1".into(),
        port: 11434,
        process_name: None,
        ai_provider: Some(crate::net::ai_traffic::provider::ProviderKind::OpenAi),
        ai_protocol: Some(crate::net::ai_traffic::provider::ModelProtocol::OpenAi),
        ..Default::default()
    };

    let mut chunk = Bytes::from("data: {\"id\":\"x\",\"choices\":[]}\n\n");
    {
        let mut ctx = ctx_for(&mut state, &conn);
        hook.on_response_chunk(&mut chunk, &mut ctx);
    }

    let stream = state
        .peek::<SseEventStream>()
        .expect("stream slot must exist for explicit provider");
    assert_eq!(stream.events.len(), 1);
}
