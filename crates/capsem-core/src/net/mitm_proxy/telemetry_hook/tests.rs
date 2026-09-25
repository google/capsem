use super::super::body::BodyStats;
use super::super::hooks::{ChunkCtx, ChunkHook, ConnMeta, HookState};
use super::*;
use crate::credential_broker::{CredentialInjection, CredentialObservation, CredentialProvider};
use crate::net::policy_config::{SecurityRuleProfile, SecurityRuleSet, SecurityRuleSource};
use capsem_logger::{credential_reference, Decision};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

async fn wait_for_db(
    db: &capsem_logger::DbWriter,
    db_path: &std::path::Path,
    mut predicate: impl FnMut(&rusqlite::Connection) -> bool,
) -> bool {
    for _ in 0..250 {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        db.flush().await;
        let conn = rusqlite::Connection::open(db_path).unwrap();
        if predicate(&conn) {
            return true;
        }
    }
    false
}

fn req_stats(preview: &[u8]) -> Arc<Mutex<BodyStats>> {
    Arc::new(Mutex::new(BodyStats {
        bytes: preview.len() as u64,
        preview: preview.to_vec(),
        max_body_capture: 64 * 1024,
    }))
}

fn ctx_for<'a>(state: &'a mut HookState, conn: &'a ConnMeta) -> ChunkCtx<'a> {
    ChunkCtx {
        state,
        conn,
        trace_id: None,
    }
}

async fn complete_response(hook: &TelemetryHook, state: &mut HookState, conn: &ConnMeta) {
    let pending = {
        let mut ctx = ctx_for(state, conn);
        hook.on_response_end(&mut ctx);
        hook.take_response_end_future(&mut ctx)
    };
    if let Some(future) = pending {
        future.await;
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

struct EnvGuard {
    // Redirects CAPSEM_HOME/RUN_DIR/ASSETS_DIR together; restores on drop.
    // None for trace-only guards, which redirect no paths.
    _capsem_paths: Option<capsem_foundation::paths::CapsemPathsGuard>,
    old_home: Option<String>,
    old_store: Option<String>,
    old_trace: Option<String>,
}

impl EnvGuard {
    fn install(capsem_home: &std::path::Path, home: &std::path::Path, test_store: &std::path::Path) -> Self {
        let old_home = std::env::var("HOME").ok();
        let old_store = std::env::var(crate::credential_broker::STORE_PATH_ENV).ok();
        let old_trace = std::env::var("CAPSEM_TRACE_ID").ok();
        std::env::set_var("HOME", home);
        std::env::set_var(crate::credential_broker::STORE_PATH_ENV, test_store);
        Self {
            _capsem_paths: Some(capsem_foundation::paths::CapsemPathsGuard::redirect(capsem_home)),
            old_home,
            old_store,
            old_trace,
        }
    }

    fn trace_only(trace_id: &str) -> Self {
        let old_home = std::env::var("HOME").ok();
        let old_store = std::env::var(crate::credential_broker::STORE_PATH_ENV).ok();
        let old_trace = std::env::var("CAPSEM_TRACE_ID").ok();
        std::env::set_var("CAPSEM_TRACE_ID", trace_id);
        Self {
            _capsem_paths: None,
            old_home,
            old_store,
            old_trace,
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.old_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        match &self.old_store {
            Some(v) => std::env::set_var(crate::credential_broker::STORE_PATH_ENV, v),
            None => std::env::remove_var(crate::credential_broker::STORE_PATH_ENV),
        }
        match &self.old_trace {
            Some(v) => std::env::set_var("CAPSEM_TRACE_ID", v),
            None => std::env::remove_var("CAPSEM_TRACE_ID"),
        }
    }
}

/// Returns a generic request context for an allowed Anthropic POST.
fn anthropic_req_ctx() -> TelemetryRequestContext {
    TelemetryRequestContext {
        domain: "api.anthropic.com".into(),
        process_name: Some("agent".into()),
        ai_provider: Some(ProviderKind::Anthropic),
        ai_protocol: Some(ModelProtocol::Anthropic),
        model_traffic: true,
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
        max_response_body_capture: 4096,
        port: 443,
        conn_type: "https-mitm",
        policy_mode: None,
        policy_action: None,
        policy_rule: None,
        policy_reason: None,
        credential_ref: None,
        credential_observations: Vec::new(),
        credential_injections: Vec::new(),
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
    assert_eq!(ev.response_body.as_deref(), Some(b"chunk-preview".as_slice()));
    assert_eq!(ev.conn_type.as_deref(), Some("https-mitm"));
}

/// The event carries each captured body once.
///
/// It used to carry every body twice -- a `*_body_preview` string and a
/// `*_body_full` string, both built from the same buffer on this hot path --
/// and nothing held the two copies to each other. The display preview is now
/// derived by the writer from the single field this asserts.
#[test]
fn build_net_event_carries_each_body_once() {
    let req_ctx = anthropic_req_ctx();
    let mut resp_stats = empty_resp_stats();
    resp_stats.preview = b"the-response".to_vec();

    let ev = build_net_event(&req_ctx, &resp_stats);
    let request_body = ev.request_body.as_deref().expect("the request body is carried");
    assert_eq!(
        request_body,
        req_ctx.request_body_stats.lock().unwrap().preview.as_slice(),
        "the request body is the captured buffer, not a re-encoded copy of it"
    );
    assert_eq!(ev.response_body.as_deref(), Some(b"the-response".as_slice()));

    // The single-source rule, asserted where a producer could break it: the
    // struct has one field per direction and this is the whole of it.
    let json = serde_json::to_value(&ev).expect("the event serializes");
    let fields = json.as_object().expect("an object");
    let body_fields: Vec<&String> = fields.keys().filter(|key| key.contains("body")).collect();
    assert_eq!(
        body_fields,
        vec!["request_body", "response_body"],
        "one body field per direction; see tests/citadel/test_ledger_body_single_source.py"
    );
}

#[test]
fn build_net_event_and_model_call_carry_credential_ref() {
    let credential_ref = credential_reference("anthropic", "sk-ant-test");
    let mut req_ctx = anthropic_req_ctx();
    req_ctx.credential_ref = Some(credential_ref.clone());
    req_ctx.credential_observations = vec![CredentialObservation {
        provider: CredentialProvider::Anthropic,
        raw_value: "sk-ant-test".to_string(),
        source: "http.header.x-api-key".to_string(),
        event_type: Some("http.request".to_string()),
        trace_id: None,
        context_json: None,
    }];
    let pricing = Arc::new(PricingTable::load());
    let trace = Arc::new(Mutex::new(TraceState::new()));

    let net = build_net_event(&req_ctx, &empty_resp_stats());
    let model = maybe_build_model_call(&req_ctx, &empty_resp_stats(), &[], &pricing, &trace)
        .expect("AI POST to /v1/messages must produce a model call");

    assert_eq!(net.credential_ref.as_deref(), Some(credential_ref.as_str()));
    assert_eq!(model.credential_ref.as_deref(), Some(credential_ref.as_str()));
    assert!(!net.credential_ref.as_deref().unwrap().contains("sk-ant-test"));
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
    req_ctx.model_traffic = false;
    let pricing = Arc::new(PricingTable::load());
    let trace = Arc::new(Mutex::new(TraceState::new()));

    let mc = maybe_build_model_call(&req_ctx, &empty_resp_stats(), &[], &pricing, &trace);
    assert!(mc.is_none());
}

#[test]
fn agy_cloudcode_stream_generate_content_is_a_model_call() {
    let mut req_ctx = anthropic_req_ctx();
    req_ctx.domain = "daily-cloudcode-pa.googleapis.com".into();
    req_ctx.process_name = Some("agy".into());
    req_ctx.ai_provider = Some(ProviderKind::Google);
    req_ctx.ai_protocol = Some(ModelProtocol::Google);
    req_ctx.path = "/v1internal:streamGenerateContent".into();
    req_ctx.request_body_stats = req_stats(b"");
    let pricing = Arc::new(PricingTable::load());
    let trace = Arc::new(Mutex::new(TraceState::new()));

    let mc = maybe_build_model_call(&req_ctx, &empty_resp_stats(), &[], &pricing, &trace)
        .expect("AGY Cloud Code streamGenerateContent should produce model telemetry");

    assert_eq!(mc.provider, "google");
    assert_eq!(mc.process_name.as_deref(), Some("agy"));
    assert_eq!(mc.path, "/v1internal:streamGenerateContent");
    assert!(mc.stream);
}

#[test]
fn google_non_streaming_function_call_is_logged_as_model_tool_call() {
    let mut req_ctx = anthropic_req_ctx();
    req_ctx.domain = "daily-cloudcode-pa.googleapis.com".into();
    req_ctx.process_name = Some("agy".into());
    req_ctx.ai_provider = Some(ProviderKind::Google);
    req_ctx.ai_protocol = Some(ModelProtocol::Google);
    req_ctx.path = "/v1internal:generateContent".into();
    req_ctx.request_body_stats = req_stats(br#"{"contents":[{"role":"user","parts":[{"text":"search"}]}]}"#);
    let response = br#"{
        "candidates": [{
            "content": {"parts": [{"functionCall": {"name": "search_web", "args": {"query": "capsem"}}}]},
            "finishReason": "STOP"
        }],
        "modelVersion": "gemini-3.1-pro-preview-customtools",
        "usageMetadata": {"promptTokenCount": 7, "candidatesTokenCount": 3}
    }"#;
    let resp_stats = TelemetryResponseStats {
        bytes: response.len() as u64,
        preview: response.to_vec(),
        max_body_capture: response.len(),
    };
    let pricing = Arc::new(PricingTable::load());
    let trace = Arc::new(Mutex::new(TraceState::new()));

    let mc = maybe_build_model_call(&req_ctx, &resp_stats, &[], &pricing, &trace)
        .expect("Google generateContent should produce model telemetry");

    assert_eq!(mc.provider, "google");
    assert_eq!(mc.model.as_deref(), Some("gemini-3.1-pro-preview-customtools"));
    assert_eq!(mc.tool_calls.len(), 1);
    assert_eq!(mc.tool_calls[0].call_id, "gemini_search_web_0");
    assert_eq!(mc.tool_calls[0].tool_name, "search_web");
    assert_eq!(mc.tool_calls[0].arguments.as_deref(), Some(r#"{"query":"capsem"}"#));
}

#[test]
fn agy_google_tool_call_survives_into_ledger_counters() {
    let mut req_ctx = anthropic_req_ctx();
    req_ctx.domain = "daily-cloudcode-pa.googleapis.com".into();
    req_ctx.process_name = Some("agy".into());
    req_ctx.ai_provider = Some(ProviderKind::Google);
    req_ctx.ai_protocol = Some(ModelProtocol::Google);
    req_ctx.path = "/v1internal:generateContent".into();
    req_ctx.request_body_stats = req_stats(br#"{"contents":[{"role":"user","parts":[{"text":"search"}]}]}"#);
    let response = br#"{
        "candidates": [{
            "content": {"parts": [{"functionCall": {"name": "search_web", "args": {"query": "capsem"}}}]},
            "finishReason": "STOP"
        }],
        "modelVersion": "gemini-3.1-pro-preview-customtools",
        "usageMetadata": {"promptTokenCount": 7, "candidatesTokenCount": 3}
    }"#;
    let resp_stats = TelemetryResponseStats {
        bytes: response.len() as u64,
        preview: response.to_vec(),
        max_body_capture: response.len(),
    };
    let pricing = Arc::new(PricingTable::load());
    let trace = Arc::new(Mutex::new(TraceState::new()));
    let model_call = maybe_build_model_call(&req_ctx, &resp_stats, &[], &pricing, &trace)
        .expect("AGY Google generateContent should produce model telemetry");

    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 8).unwrap();
    writer.write_blocking(capsem_logger::WriteOp::ModelCall(model_call));
    writer.shutdown_blocking();

    // Counted the way every surface reads it: the writer's committed snapshot.
    let counters = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            capsem_logger::DbHandle::open_external_reader(&db_path)
                .unwrap()
                .ledger_counters()
                .await
        })
        .unwrap();
    assert_eq!(counters.model.total.calls, 1);
    assert_eq!(counters.tools.calls, 1);
    assert_eq!(counters.tools.by_tool.len(), 1);
    assert_eq!(counters.tools.by_tool["search_web"].calls, 1);

    let reader = capsem_logger::DbReader::open(&db_path).unwrap();
    let calls = reader.recent_model_calls(1).unwrap();
    assert_eq!(calls.len(), 1);
    let tool_rows = reader.tool_calls_for(calls[0].0).unwrap();
    assert_eq!(tool_rows.len(), 1);
    assert_eq!(tool_rows[0].call_id, "gemini_search_web_0");
    assert_eq!(tool_rows[0].tool_name, "search_web");
    assert_eq!(tool_rows[0].arguments.as_deref(), Some(r#"{"query":"capsem"}"#));
}

#[test]
fn openai_non_streaming_tool_call_carries_request_trace() {
    let _trace_guard = EnvGuard::trace_only("feedfacecafebeef");
    let mut req_ctx = anthropic_req_ctx();
    req_ctx.domain = "127.0.0.1".into();
    req_ctx.ai_provider = Some(ProviderKind::OpenAi);
    req_ctx.ai_protocol = Some(ModelProtocol::OpenAi);
    req_ctx.path = "/v1/chat/completions".into();
    req_ctx.request_body_stats = req_stats(br#"{"model":"mock-local","messages":[{"role":"user","content":"hello"}]}"#);
    let response = br#"{
        "id": "chatcmpl-mock-local",
        "object": "chat.completion",
        "model": "mock-local",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "hello from capsem-mock-server",
                "tool_calls": [{
                    "id": "tool_0001",
                    "type": "function",
                    "function": {
                        "name": "fixture_lookup",
                        "arguments": "{\"query\":\"capsem\"}"
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": {
            "prompt_tokens": 7,
            "completion_tokens": 5,
            "total_tokens": 12
        }
    }"#;
    let resp_stats = TelemetryResponseStats {
        bytes: response.len() as u64,
        preview: response.to_vec(),
        max_body_capture: response.len(),
    };
    let pricing = Arc::new(PricingTable::load());
    let trace = Arc::new(Mutex::new(TraceState::new()));
    let model_call = maybe_build_model_call(&req_ctx, &resp_stats, &[], &pricing, &trace)
        .expect("OpenAI-compatible chat completion should produce model telemetry");

    assert_ne!(model_call.trace_id.as_deref(), Some("feedfacecafebeef"));
    assert!(model_call.trace_id.as_deref().is_some_and(|trace| !trace.is_empty()));
    assert_eq!(model_call.provider, "openai");
    assert_eq!(model_call.model.as_deref(), Some("mock-local"));
    assert_eq!(
        model_call.text_content.as_deref(),
        Some("hello from capsem-mock-server")
    );
    assert_eq!(model_call.stop_reason.as_deref(), Some("tool_use"));
    assert_eq!(model_call.input_tokens, Some(7));
    assert_eq!(model_call.output_tokens, Some(5));
    assert_eq!(model_call.tool_calls.len(), 1);
    assert_eq!(model_call.tool_calls[0].call_id, "tool_0001");
    assert_eq!(model_call.tool_calls[0].tool_name, "fixture_lookup");
    assert_eq!(
        model_call.tool_calls[0].arguments.as_deref(),
        Some(r#"{"query":"capsem"}"#)
    );
    assert_eq!(
        model_call.tool_calls[0].trace_id.as_deref(),
        model_call.trace_id.as_deref()
    );
}

#[test]
fn ollama_endpoint_can_use_anthropic_wire_protocol() {
    let mut req_ctx = anthropic_req_ctx();
    req_ctx.domain = "127.0.0.1".into();
    req_ctx.port = 11434;
    req_ctx.ai_provider = Some(ProviderKind::Ollama);
    req_ctx.ai_protocol = Some(ModelProtocol::Anthropic);
    req_ctx.path = "/v1/messages".into();
    req_ctx.request_body_stats =
        req_stats(br#"{"model":"gemma4:latest","max_tokens":128,"messages":[{"role":"user","content":"hello"}]}"#);
    let response = br#"{
        "id": "msg_ironbank",
        "type": "message",
        "role": "assistant",
        "model": "gemma4:latest",
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 11, "output_tokens": 7},
        "content": [
            {"type": "thinking", "thinking": "launcher reasoning"},
            {"type": "text", "text": "launcher response"}
        ]
    }"#;
    let resp_stats = TelemetryResponseStats {
        bytes: response.len() as u64,
        preview: response.to_vec(),
        max_body_capture: response.len(),
    };
    let pricing = Arc::new(PricingTable::load());
    let trace = Arc::new(Mutex::new(TraceState::new()));
    let model_call = maybe_build_model_call(&req_ctx, &resp_stats, &[], &pricing, &trace)
        .expect("Ollama endpoint serving Anthropic protocol should produce model telemetry");

    assert_eq!(model_call.provider, "ollama");
    assert_eq!(model_call.path, "/v1/messages");
    assert_eq!(model_call.model.as_deref(), Some("gemma4:latest"));
    assert_eq!(model_call.text_content.as_deref(), Some("launcher response"));
    assert_eq!(model_call.thinking_content.as_deref(), Some("launcher reasoning"));
    assert_eq!(model_call.stop_reason.as_deref(), Some("end_turn"));
    assert_eq!(model_call.input_tokens, Some(11));
    assert_eq!(model_call.output_tokens, Some(7));
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

    let req_ctx = anthropic_req_ctx();
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
    let mc1 = maybe_build_model_call(&req1, &empty_resp_stats(), &events1, &pricing, &trace).expect("model call");
    assert_eq!(mc1.stop_reason.as_deref(), Some("tool_use"));
    assert_eq!(mc1.tool_calls.len(), 1);
    assert_eq!(mc1.tool_calls[0].call_id, "call_x");
    let trace_a = mc1.trace_id.expect("trace assigned");

    // Second call: client sends back a tool_response for 'call_x'.
    // Body parsed from `request_body_stats.preview`; we craft an Anthropic
    // tool_result with matching call_id.
    let req2 = TelemetryRequestContext {
        request_body_stats: req_stats(
            br#"{"messages":[{"role":"user","content":[{"type":"tool_result","tool_use_id":"call_x","content":"ok"}]}]}"#,
        ),
        ..anthropic_req_ctx()
    };
    let mc2 = maybe_build_model_call(&req2, &empty_resp_stats(), &[], &pricing, &trace).expect("model call");
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
        security_rules: empty_security_rules(),
        plugin_policy: Arc::new(std::sync::RwLock::new(BTreeMap::new().into())),
    })
}

fn empty_security_rules() -> Arc<std::sync::RwLock<Arc<SecurityRuleSet>>> {
    Arc::new(std::sync::RwLock::new(Arc::new(SecurityRuleSet::new(Vec::new()))))
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
        assert!(hook.take_response_end_future(&mut ctx).is_none());
    }

    // Hook must not have allocated a response stats slot.
    assert!(state.peek::<TelemetryResponseStats>().is_none());
}

#[test]
fn telemetry_completion_must_not_block_on_security_ledger_writes() {
    let source = include_str!("../telemetry_hook.rs");
    let completion = source
        .split("fn on_response_end(&self, ctx: &mut ChunkCtx<'_>)")
        .nth(1)
        .expect("telemetry completion hook must exist")
        .split("/// Pure builder: assembles a `NetEvent`")
        .next()
        .expect("telemetry completion hook body must be bounded");

    for forbidden in [
        "emit_security_write_blocking",
        "emit_security_write_try",
        "emit_matching_security_rules_for_evaluated_event_blocking",
        "emit_matching_security_rules_blocking",
    ] {
        assert!(
            !completion.contains(forbidden),
            "MITM telemetry completion must not call {forbidden}. The HTTP response path must enqueue/spawn ledger work asynchronously; blocking forensic JSON/SQLite work caused route latency collapse under tiny HTTP load."
        );
    }
    assert!(
        completion.contains("PendingTelemetryCompletion"),
        "MITM telemetry completion must hand bounded logger work to the body-owned async EOF barrier"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn hook_accepts_primary_net_event_before_completing_response() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let db = Arc::new(DbWriter::open(&db_path, 64).expect("test db"));
    let deps = Arc::new(TelemetryDeps {
        db: Arc::clone(&db),
        pricing: Arc::new(PricingTable::load()),
        trace_state: Arc::new(Mutex::new(TraceState::new())),
        security_rules: empty_security_rules(),
        plugin_policy: Arc::new(std::sync::RwLock::new(BTreeMap::new().into())),
    });
    let hook = TelemetryHook::new(deps);

    let mut req_ctx = anthropic_req_ctx();
    req_ctx.domain = "127.0.0.1".to_string();
    req_ctx.ai_provider = None;
    req_ctx.ai_protocol = None;
    req_ctx.model_traffic = false;
    req_ctx.method = "GET".to_string();
    req_ctx.path = "/bytes/10mb".to_string();
    req_ctx.status_code = Some(200);
    req_ctx.decision = Decision::Allowed;
    req_ctx.process_name = Some("curl".to_string());

    let mut state = HookState::default();
    let conn = ConnMeta {
        domain: "127.0.0.1".to_string(),
        port: 443,
        process_name: Some("curl".to_string()),
        ..Default::default()
    };
    {
        let mut c = ctx_for(&mut state, &conn);
        *c.state::<Option<TelemetryRequestContext>>(|| None) = Some(req_ctx);
        *c.state::<TelemetryResponseStats>(TelemetryResponseStats::default) = TelemetryResponseStats {
            bytes: 10 * 1024 * 1024,
            preview: Vec::new(),
            max_body_capture: 0,
        };
    }

    complete_response(&hook, &mut state, &conn).await;

    db.shutdown_blocking();
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let row: (String, String, u64) = conn
        .query_row(
            "SELECT domain, path, bytes_received FROM net_events WHERE process_name = 'curl'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("primary net event must be accepted before hook returns");
    assert_eq!(
        row,
        ("127.0.0.1".to_string(), "/bytes/10mb".to_string(), 10 * 1024 * 1024)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn bounded_logger_backpressure_keeps_every_completed_response_event() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let db = Arc::new(DbWriter::open(&db_path, 1).expect("test db"));
    let hook = TelemetryHook::new(Arc::new(TelemetryDeps {
        db: Arc::clone(&db),
        pricing: Arc::new(PricingTable::load()),
        trace_state: Arc::new(Mutex::new(TraceState::new())),
        security_rules: empty_security_rules(),
        plugin_policy: Arc::new(std::sync::RwLock::new(BTreeMap::new().into())),
    }));
    let conn = ConnMeta {
        domain: "127.0.0.1".to_string(),
        port: 443,
        process_name: Some("capsem-bench-rs".to_string()),
        ..Default::default()
    };
    let mut completions = Vec::new();

    for index in 0..512 {
        let mut req_ctx = anthropic_req_ctx();
        req_ctx.domain = "127.0.0.1".to_string();
        req_ctx.ai_provider = None;
        req_ctx.ai_protocol = None;
        req_ctx.model_traffic = false;
        req_ctx.path = format!("/bounded/{index}");
        req_ctx.process_name = Some("capsem-bench-rs".to_string());
        let mut state = HookState::default();
        let completion = {
            let mut ctx = ctx_for(&mut state, &conn);
            *ctx.state::<Option<TelemetryRequestContext>>(|| None) = Some(req_ctx);
            hook.on_response_end(&mut ctx);
            hook.take_response_end_future(&mut ctx)
                .expect("seeded telemetry must return async completion work")
        };
        completions.push(completion);
    }

    futures::future::join_all(completions).await;
    db.flush().await;
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let count: usize = conn
        .query_row(
            "SELECT COUNT(*) FROM net_events WHERE domain = '127.0.0.1'",
            [],
            |row| row.get(0),
        )
        .unwrap();

    assert_eq!(count, 512);
    db.shutdown_blocking();
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

/// The credential-broker and security-rule ledger writes.
///
/// Their own module because they are a different kind of test from the ones
/// above: those classify one request and assert the event it builds, while
/// these stand up a broker, a rule set and a real `DbWriter` and then read the
/// ledger back. Splitting them is also what keeps this file inside the Rust
/// shape ceiling -- see `config/gate.toml [boundary]`.
mod decision;
mod headers;
mod ledger;
