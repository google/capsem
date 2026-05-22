use super::super::body::BodyStats;
use super::super::hooks::{ChunkCtx, ChunkHook, ConnMeta, HookState};
use super::*;
use capsem_logger::Decision;
use std::sync::{Arc, Mutex};
use std::time::Instant;

fn req_stats(preview: &[u8]) -> Arc<Mutex<BodyStats>> {
    Arc::new(Mutex::new(BodyStats {
        bytes: preview.len() as u64,
        preview: preview.to_vec(),
        max_preview: 64 * 1024,
    }))
}

fn ctx_for<'a>(state: &'a mut HookState, conn: &'a ConnMeta) -> ChunkCtx<'a> {
    ChunkCtx {
        state,
        conn,
        trace_id: None,
    }
}

fn any_conn() -> ConnMeta {
    ConnMeta {
        domain: "api.anthropic.com".into(),
        port: 443,
        process_name: None,
        ..Default::default()
    }
}

/// Returns a generic request context for an allowed Anthropic POST.
fn anthropic_req_ctx() -> TelemetryRequestContext {
    TelemetryRequestContext {
        domain: "api.anthropic.com".into(),
        process_name: Some("agent".into()),
        ai_provider: Some(ProviderKind::Anthropic),
        method: "POST".into(),
        path: "/v1/messages".into(),
        query: None,
        status_code: Some(200),
        decision: Decision::Allowed,
        matched_rule: Some("default-dev-allow".into()),
        request_headers: Some("host: api.anthropic.com".into()),
        response_headers: Some("content-type: text/event-stream".into()),
        start_time: Instant::now(),
        request_body_stats: req_stats(b"{\"model\":\"claude-test\",\"messages\":[]}"),
        max_response_preview: 4096,
        port: 443,
        conn_type: "https-mitm",
        identity: TelemetryIdentityContext::default(),
        policy_mode: None,
        policy_action: None,
        policy_rule: None,
        policy_reason: None,
        runtime_security_result: None,
    }
}

fn empty_resp_stats() -> TelemetryResponseStats {
    TelemetryResponseStats::default()
}

/// `build_net_event` populates the basic fields straight from the
/// context.
#[test]
fn build_net_event_carries_request_fields() {
    let req_ctx = anthropic_req_ctx();
    let mut resp_stats = empty_resp_stats();
    resp_stats.bytes = 4567;
    resp_stats.preview = b"chunk-preview".to_vec();

    let ev = build_net_event(&req_ctx, &resp_stats);
    assert_eq!(ev.domain, "api.anthropic.com");
    assert_eq!(ev.method.as_deref(), Some("POST"));
    assert_eq!(ev.path.as_deref(), Some("/v1/messages"));
    assert_eq!(ev.status_code, Some(200));
    assert_eq!(ev.decision, Decision::Allowed);
    assert_eq!(ev.bytes_sent, 37); // length of the seeded preview bytes
    assert_eq!(ev.bytes_received, 4567);
    assert_eq!(ev.response_body_preview.as_deref(), Some("chunk-preview"));
    assert_eq!(ev.conn_type.as_deref(), Some("https-mitm"));
}

#[test]
fn build_http_resolved_security_event_carries_http_subject_and_allow_action() {
    let req_ctx = anthropic_req_ctx();
    let mut resp_stats = empty_resp_stats();
    resp_stats.bytes = 4567;
    resp_stats.preview = b"chunk-preview".to_vec();
    let net_event = build_net_event(&req_ctx, &resp_stats);

    let resolved = build_http_resolved_security_event(&req_ctx, &resp_stats, &net_event);

    assert_eq!(resolved.event.common.event_type, "http.request");
    assert_eq!(
        resolved.event.common.trace_id.as_deref(),
        net_event.trace_id.as_deref()
    );
    assert_eq!(resolved.event.common.source_engine, SourceEngine::Network);
    assert_eq!(
        resolved.event.common.attribution_scope,
        AiAttributionScope::Vm
    );
    assert!(matches!(
        resolved.final_action,
        capsem_security_engine::SecurityAction::Continue
    ));
    let capsem_security_engine::SecurityEventSubject::Http(subject) = &resolved.event.subject
    else {
        panic!("expected http subject");
    };
    assert_eq!(subject.method, "POST");
    assert_eq!(subject.host, "api.anthropic.com");
    assert_eq!(subject.port, Some(443));
    assert_eq!(subject.path.as_deref(), Some("/v1/messages"));
    assert_eq!(
        subject.url.as_deref(),
        Some("https://api.anthropic.com/v1/messages")
    );
    assert_eq!(subject.request_bytes, 37);
    assert_eq!(subject.response_status, Some(200));
    assert_eq!(subject.response_bytes, Some(4567));
    assert_eq!(
        subject
            .response_body
            .as_ref()
            .and_then(|body| body.text.as_deref()),
        Some("chunk-preview")
    );
}

#[test]
fn build_http_resolved_security_event_carries_vm_profile_and_user_identity() {
    let mut req_ctx = anthropic_req_ctx();
    req_ctx.identity = TelemetryIdentityContext {
        vm_id: Some("vm-winterfell".into()),
        session_id: Some("session-winterfell".into()),
        profile_id: Some("coding".into()),
        profile_revision: Some("2026.0522.1".into()),
        user_id: Some("arya".into()),
    };
    let resp_stats = empty_resp_stats();
    let net_event = build_net_event(&req_ctx, &resp_stats);

    let resolved = build_http_resolved_security_event(&req_ctx, &resp_stats, &net_event);

    assert_eq!(
        resolved.event.common.vm_id.as_deref(),
        Some("vm-winterfell")
    );
    assert_eq!(
        resolved.event.common.session_id.as_deref(),
        Some("session-winterfell")
    );
    assert_eq!(resolved.event.common.profile_id.as_deref(), Some("coding"));
    assert_eq!(
        resolved.event.common.profile_revision.as_deref(),
        Some("2026.0522.1")
    );
    assert_eq!(resolved.event.common.user_id.as_deref(), Some("arya"));
}

#[test]
fn build_http_resolved_security_event_maps_denied_network_decision_to_block() {
    let mut req_ctx = anthropic_req_ctx();
    req_ctx.decision = Decision::Denied;
    req_ctx.status_code = Some(403);
    req_ctx.matched_rule = Some("runtime.block_metadata".into());
    req_ctx.policy_rule = Some("policy.http.block_metadata".into());
    req_ctx.policy_reason = Some("metadata access".into());
    let resp_stats = empty_resp_stats();
    let net_event = build_net_event(&req_ctx, &resp_stats);

    let resolved = build_http_resolved_security_event(&req_ctx, &resp_stats, &net_event);

    assert!(matches!(
        resolved.final_action,
        capsem_security_engine::SecurityAction::Block(_)
    ));
    assert_eq!(
        resolved
            .event
            .decision
            .as_ref()
            .and_then(|d| d.rule.as_deref()),
        Some("policy.http.block_metadata")
    );
    assert_eq!(resolved.steps.len(), 1);
    assert_eq!(
        resolved.steps[0].kind,
        capsem_security_engine::ResolvedEventStepKind::EnforcementMatch
    );
    assert_eq!(
        resolved.steps[0].status,
        capsem_security_engine::StepStatus::Matched
    );
}

/// HEAD request to an AI domain is *not* a model call (probe).
#[test]
fn head_request_is_not_a_model_call() {
    let mut req_ctx = anthropic_req_ctx();
    req_ctx.method = "HEAD".into();
    let pricing = Arc::new(PricingTable::load());
    let trace = Arc::new(Mutex::new(TraceState::new()));

    let mc = maybe_build_model_call(&req_ctx, &empty_resp_stats(), &[], &pricing, &trace);
    assert!(mc.is_none());
}

/// Non-LLM API path (e.g. `/v1/models`) is not a model call.
#[test]
fn non_llm_path_is_not_a_model_call() {
    let mut req_ctx = anthropic_req_ctx();
    req_ctx.path = "/v1/models".into();
    let pricing = Arc::new(PricingTable::load());
    let trace = Arc::new(Mutex::new(TraceState::new()));

    let mc = maybe_build_model_call(&req_ctx, &empty_resp_stats(), &[], &pricing, &trace);
    assert!(mc.is_none());
}

/// Non-AI provider returns no model call.
#[test]
fn non_ai_provider_is_not_a_model_call() {
    let mut req_ctx = anthropic_req_ctx();
    req_ctx.ai_provider = None;
    req_ctx.domain = "example.com".into();
    let pricing = Arc::new(PricingTable::load());
    let trace = Arc::new(Mutex::new(TraceState::new()));

    let mc = maybe_build_model_call(&req_ctx, &empty_resp_stats(), &[], &pricing, &trace);
    assert!(mc.is_none());
}

/// LlmEvents from the interpreter chain feed into the model call's
/// `text_content` / `tool_calls` / `stop_reason`.
#[test]
fn llm_events_flow_into_model_call() {
    use crate::net::ai_traffic::events::{LlmEvent, StopReason};

    let mut req_ctx = anthropic_req_ctx();
    req_ctx.identity = TelemetryIdentityContext {
        vm_id: Some("vm-ai".into()),
        session_id: Some("session-ai".into()),
        profile_id: Some("coding".into()),
        profile_revision: Some("2026.0522.1".into()),
        user_id: Some("bran".into()),
    };
    let pricing = Arc::new(PricingTable::load());
    let trace = Arc::new(Mutex::new(TraceState::new()));
    let events = vec![
        LlmEvent::MessageStart {
            message_id: Some("msg_1".into()),
            model: Some("claude-test".into()),
        },
        LlmEvent::TextDelta {
            index: 0,
            text: "hello".into(),
        },
        LlmEvent::MessageEnd {
            stop_reason: Some(StopReason::EndTurn),
        },
    ];
    let mc = maybe_build_model_call(&req_ctx, &empty_resp_stats(), &events, &pricing, &trace)
        .expect("AI POST to /v1/messages must produce a model call");
    assert_eq!(mc.provider, "anthropic");
    assert_eq!(mc.model.as_deref(), Some("claude-test"));
    assert_eq!(mc.text_content.as_deref(), Some("hello"));
    assert_eq!(mc.stop_reason.as_deref(), Some("end_turn"));
    assert_eq!(mc.message_id.as_deref(), Some("msg_1"));
    let evidence = mc.ai_evidence.as_ref().expect("canonical AI evidence");
    assert_eq!(evidence.trace_id, mc.trace_id.as_deref().unwrap());
    assert_eq!(evidence.provider.as_str(), "anthropic");
    assert!(evidence
        .request
        .request_id
        .starts_with(&format!("request:{}:", evidence.trace_id)));
    assert_eq!(evidence.source_engine, SourceEngine::Network);
    assert_eq!(evidence.attribution_scope, AiAttributionScope::Vm);
    assert_eq!(evidence.origin_kind, AiOriginKind::GuestNetwork);
    assert_eq!(evidence.vm_id.as_deref(), Some("vm-ai"));
    assert_eq!(evidence.session_id.as_deref(), Some("session-ai"));
    assert_eq!(evidence.profile_id.as_deref(), Some("coding"));
    assert_eq!(evidence.user_id.as_deref(), Some("bran"));
    assert_eq!(
        evidence
            .response
            .as_ref()
            .and_then(|r| r.text_preview.as_deref()),
        Some("hello")
    );
}

/// Tool-use stop reason registers tool_call IDs in the trace state so
/// the next request's tool_responses can resolve back to the same
/// trace_id.
#[test]
fn tool_use_chains_traces_across_requests() {
    use crate::net::ai_traffic::events::{LlmEvent, StopReason};
    let pricing = Arc::new(PricingTable::load());
    let trace = Arc::new(Mutex::new(TraceState::new()));

    // First call: model emits a tool_use, with tool_call_id 'call_x'.
    let req1 = anthropic_req_ctx();
    let events1 = vec![
        LlmEvent::ToolCallStart {
            index: 0,
            call_id: "call_x".into(),
            name: "list_files".into(),
        },
        LlmEvent::ContentBlockEnd { index: 0 },
        LlmEvent::MessageEnd {
            stop_reason: Some(StopReason::ToolUse),
        },
    ];
    let mc1 = maybe_build_model_call(&req1, &empty_resp_stats(), &events1, &pricing, &trace)
        .expect("model call");
    assert_eq!(mc1.stop_reason.as_deref(), Some("tool_use"));
    assert_eq!(mc1.tool_calls.len(), 1);
    assert_eq!(mc1.tool_calls[0].call_id, "call_x");
    let trace_a = mc1.trace_id.clone().expect("trace assigned");

    // Second call: client sends back a tool_response for 'call_x'.
    // Body parsed from `request_body_stats.preview`; we craft an Anthropic
    // tool_result with matching call_id.
    let req2 = TelemetryRequestContext {
        request_body_stats: req_stats(
            br#"{"messages":[{"role":"user","content":[{"type":"tool_result","tool_use_id":"call_x","content":"ok"}]}]}"#,
        ),
        ..anthropic_req_ctx()
    };
    let mc2 = maybe_build_model_call(&req2, &empty_resp_stats(), &[], &pricing, &trace)
        .expect("model call");
    assert_eq!(mc2.trace_id, Some(trace_a));
}

// ── ChunkHook surface ─────────────────────────────────────────────

fn fake_deps() -> Arc<TelemetryDeps> {
    // In-memory DbWriter is fine -- we don't actually inspect writes
    // here; the pure builders are tested above.
    let db = Arc::new(DbWriter::open_in_memory(64).expect("in-memory db"));
    Arc::new(TelemetryDeps {
        db,
        pricing: Arc::new(PricingTable::load()),
        trace_state: Arc::new(Mutex::new(TraceState::new())),
    })
}

fn deps_with_db(db: Arc<DbWriter>) -> Arc<TelemetryDeps> {
    Arc::new(TelemetryDeps {
        db,
        pricing: Arc::new(PricingTable::load()),
        trace_state: Arc::new(Mutex::new(TraceState::new())),
    })
}

/// Without a seeded request context, the hook is shadow-mode: it
/// doesn't allocate the response stats slot and doesn't emit on end.
#[test]
fn shadow_mode_when_request_context_unseeded() {
    let hook = TelemetryHook::new(fake_deps());
    let mut state = HookState::default();
    let conn = any_conn();
    let mut chunk = Bytes::from_static(b"hello world");

    {
        let mut ctx = ctx_for(&mut state, &conn);
        hook.on_response_chunk(&mut chunk, &mut ctx);
    }
    {
        let mut ctx = ctx_for(&mut state, &conn);
        hook.on_response_end(&mut ctx);
    }

    // Hook must not have allocated a response stats slot.
    assert!(state.peek::<TelemetryResponseStats>().is_none());
}

/// With a seeded request context, the hook tallies bytes + preview
/// across chunks.
#[tokio::test]
async fn chunk_counting_with_seeded_context() {
    let hook = TelemetryHook::new(fake_deps());
    let mut state = HookState::default();
    let conn = any_conn();

    // Seed the request context as `Some(ctx)` -- the hook reads from
    // the slot via `ctx.state::<Option<TelemetryRequestContext>>()`.
    {
        let mut c = ChunkCtx {
            state: &mut state,
            conn: &conn,
            trace_id: None,
        };
        let slot = c.state::<Option<TelemetryRequestContext>>(|| None);
        *slot = Some(anthropic_req_ctx());
    }

    let mut a = Bytes::from_static(b"hello ");
    let mut b = Bytes::from_static(b"world");
    {
        let mut ctx = ctx_for(&mut state, &conn);
        hook.on_response_chunk(&mut a, &mut ctx);
    }
    {
        let mut ctx = ctx_for(&mut state, &conn);
        hook.on_response_chunk(&mut b, &mut ctx);
    }

    let stats = state.peek::<TelemetryResponseStats>().expect("stats slot");
    assert_eq!(stats.bytes, 11);
    assert_eq!(stats.preview, b"hello world");
}

#[tokio::test]
async fn response_end_writes_net_event_and_resolved_security_event() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let db = Arc::new(DbWriter::open(&db_path, 64).expect("db writer"));
    let hook = TelemetryHook::new(deps_with_db(Arc::clone(&db)));
    let mut state = HookState::default();
    let conn = any_conn();

    {
        let mut c = ChunkCtx {
            state: &mut state,
            conn: &conn,
            trace_id: None,
        };
        let slot = c.state::<Option<TelemetryRequestContext>>(|| None);
        *slot = Some(anthropic_req_ctx());
    }

    let mut chunk = Bytes::from_static(b"ok");
    {
        let mut ctx = ctx_for(&mut state, &conn);
        hook.on_response_chunk(&mut chunk, &mut ctx);
    }
    {
        let mut ctx = ctx_for(&mut state, &conn);
        hook.on_response_end(&mut ctx);
    }
    tokio::task::yield_now().await;
    db.shutdown_blocking();

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let net_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM net_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(net_count, 1);
    let security_row: (String, String, String, String) = conn
        .query_row(
            "SELECT event_family, event_type, source_engine, final_action FROM security_events",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        security_row,
        (
            "http".to_string(),
            "http.request".to_string(),
            "network".to_string(),
            "continue".to_string(),
        )
    );
}
