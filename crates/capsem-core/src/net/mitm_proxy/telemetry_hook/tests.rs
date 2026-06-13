use super::super::body::BodyStats;
use super::super::hooks::{ChunkCtx, ChunkHook, ConnMeta, HookState};
use super::*;
use crate::credential_broker::{CredentialInjection, CredentialObservation, CredentialProvider};
use crate::net::policy_config::{SecurityRuleProfile, SecurityRuleSet, SecurityRuleSource};
use capsem_logger::{credential_reference, Decision};
use std::collections::BTreeMap;
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

struct EnvGuard {
    old_home_override: Option<String>,
    old_home: Option<String>,
    old_store: Option<String>,
    old_trace: Option<String>,
}

impl EnvGuard {
    fn install(
        capsem_home: &std::path::Path,
        home: &std::path::Path,
        test_store: &std::path::Path,
    ) -> Self {
        let old_home_override = std::env::var("CAPSEM_HOME").ok();
        let old_home = std::env::var("HOME").ok();
        let old_store = std::env::var(crate::credential_broker::TEST_STORE_ENV).ok();
        let old_trace = std::env::var("CAPSEM_TRACE_ID").ok();
        std::env::set_var("CAPSEM_HOME", capsem_home);
        std::env::set_var("HOME", home);
        std::env::set_var(crate::credential_broker::TEST_STORE_ENV, test_store);
        Self {
            old_home_override,
            old_home,
            old_store,
            old_trace,
        }
    }

    fn trace_only(trace_id: &str) -> Self {
        let old_home_override = std::env::var("CAPSEM_HOME").ok();
        let old_home = std::env::var("HOME").ok();
        let old_store = std::env::var(crate::credential_broker::TEST_STORE_ENV).ok();
        let old_trace = std::env::var("CAPSEM_TRACE_ID").ok();
        std::env::set_var("CAPSEM_TRACE_ID", trace_id);
        Self {
            old_home_override,
            old_home,
            old_store,
            old_trace,
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.old_home_override {
            Some(v) => std::env::set_var("CAPSEM_HOME", v),
            None => std::env::remove_var("CAPSEM_HOME"),
        }
        match &self.old_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        match &self.old_store {
            Some(v) => std::env::set_var(crate::credential_broker::TEST_STORE_ENV, v),
            None => std::env::remove_var(crate::credential_broker::TEST_STORE_ENV),
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
        max_response_preview: 4096,
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
    assert_eq!(ev.response_body_preview.as_deref(), Some("chunk-preview"));
    assert_eq!(ev.conn_type.as_deref(), Some("https-mitm"));
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
        confidence: 1.0,
        trace_id: None,
        context_json: None,
    }];
    let pricing = Arc::new(PricingTable::load());
    let trace = Arc::new(Mutex::new(TraceState::new()));

    let net = build_net_event(&req_ctx, &empty_resp_stats());
    let model = maybe_build_model_call(&req_ctx, &empty_resp_stats(), &[], &pricing, &trace)
        .expect("AI POST to /v1/messages must produce a model call");

    assert_eq!(net.credential_ref.as_deref(), Some(credential_ref.as_str()));
    assert_eq!(
        model.credential_ref.as_deref(),
        Some(credential_ref.as_str())
    );
    assert!(!net
        .credential_ref
        .as_deref()
        .unwrap()
        .contains("sk-ant-test"));
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
    req_ctx.path = "/v1internal:generateContent".into();
    req_ctx.request_body_stats =
        req_stats(br#"{"contents":[{"role":"user","parts":[{"text":"search"}]}]}"#);
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
        max_preview: response.len(),
    };
    let pricing = Arc::new(PricingTable::load());
    let trace = Arc::new(Mutex::new(TraceState::new()));

    let mc = maybe_build_model_call(&req_ctx, &resp_stats, &[], &pricing, &trace)
        .expect("Google generateContent should produce model telemetry");

    assert_eq!(mc.provider, "google");
    assert_eq!(
        mc.model.as_deref(),
        Some("gemini-3.1-pro-preview-customtools")
    );
    assert_eq!(mc.tool_calls.len(), 1);
    assert_eq!(mc.tool_calls[0].call_id, "gemini_search_web_0");
    assert_eq!(mc.tool_calls[0].tool_name, "search_web");
    assert_eq!(
        mc.tool_calls[0].arguments.as_deref(),
        Some(r#"{"query":"capsem"}"#)
    );
}

#[test]
fn agy_google_tool_call_survives_into_session_stats() {
    let mut req_ctx = anthropic_req_ctx();
    req_ctx.domain = "daily-cloudcode-pa.googleapis.com".into();
    req_ctx.process_name = Some("agy".into());
    req_ctx.ai_provider = Some(ProviderKind::Google);
    req_ctx.path = "/v1internal:generateContent".into();
    req_ctx.request_body_stats =
        req_stats(br#"{"contents":[{"role":"user","parts":[{"text":"search"}]}]}"#);
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
        max_preview: response.len(),
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

    let reader = capsem_logger::DbReader::open(&db_path).unwrap();
    let stats = reader.session_stats().unwrap();
    assert_eq!(stats.model_call_count, 1);
    assert_eq!(stats.total_tool_calls, 1);

    let usage = reader.tool_usage_frequency(10).unwrap();
    assert_eq!(usage.len(), 1);
    assert_eq!(usage[0].tool_name, "search_web");
    assert_eq!(usage[0].count, 1);

    let calls = reader.recent_model_calls(1).unwrap();
    assert_eq!(calls.len(), 1);
    let tool_rows = reader.tool_calls_for(calls[0].0).unwrap();
    assert_eq!(tool_rows.len(), 1);
    assert_eq!(tool_rows[0].call_id, "gemini_search_web_0");
    assert_eq!(tool_rows[0].tool_name, "search_web");
    assert_eq!(
        tool_rows[0].arguments.as_deref(),
        Some(r#"{"query":"capsem"}"#)
    );
}

#[test]
fn openai_non_streaming_tool_call_carries_request_trace() {
    let _trace_guard = EnvGuard::trace_only("feedfacecafebeef");
    let mut req_ctx = anthropic_req_ctx();
    req_ctx.domain = "127.0.0.1".into();
    req_ctx.ai_provider = Some(ProviderKind::OpenAi);
    req_ctx.path = "/v1/chat/completions".into();
    req_ctx.request_body_stats =
        req_stats(br#"{"model":"mock-local","messages":[{"role":"user","content":"hello"}]}"#);
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
        max_preview: response.len(),
    };
    let pricing = Arc::new(PricingTable::load());
    let trace = Arc::new(Mutex::new(TraceState::new()));
    let model_call = maybe_build_model_call(&req_ctx, &resp_stats, &[], &pricing, &trace)
        .expect("OpenAI-compatible chat completion should produce model telemetry");

    assert_eq!(model_call.trace_id.as_deref(), Some("feedfacecafebeef"));
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
        Some("feedfacecafebeef")
    );
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
        security_rules: empty_security_rules(),
        plugin_policy: Arc::new(std::sync::RwLock::new(BTreeMap::new())),
    })
}

fn empty_security_rules() -> Arc<std::sync::RwLock<Arc<SecurityRuleSet>>> {
    Arc::new(std::sync::RwLock::new(Arc::new(SecurityRuleSet::new(
        Vec::new(),
    ))))
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
async fn hook_writes_substitution_event_and_shared_credential_ref() {
    let _lock = crate::credential_broker::TEST_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let capsem_home = dir.path().join("capsem-home");
    let test_store = dir.path().join("credential-store.json");
    let _guard = EnvGuard::install(&capsem_home, dir.path(), &test_store);

    let db = Arc::new(DbWriter::open(&db_path, 64).expect("test db"));
    let deps = Arc::new(TelemetryDeps {
        db: Arc::clone(&db),
        pricing: Arc::new(PricingTable::load()),
        trace_state: Arc::new(Mutex::new(TraceState::new())),
        security_rules: empty_security_rules(),
        plugin_policy: Arc::new(std::sync::RwLock::new(BTreeMap::new())),
    });
    let hook = TelemetryHook::new(deps);
    let raw = "sk-ant-hook-test";
    let credential_ref = credential_reference("anthropic", raw);
    let mut req_ctx = anthropic_req_ctx();
    req_ctx.credential_ref = Some(credential_ref.clone());
    req_ctx.credential_observations = vec![CredentialObservation {
        provider: CredentialProvider::Anthropic,
        raw_value: raw.to_string(),
        source: "http.header.x-api-key".to_string(),
        event_type: Some("http.request".to_string()),
        confidence: 1.0,
        trace_id: Some("trace-hook".to_string()),
        context_json: Some(r#"{"domain":"api.anthropic.com"}"#.to_string()),
    }];

    let mut state = HookState::default();
    let conn = any_conn();
    {
        let mut c = ctx_for(&mut state, &conn);
        *c.state::<Option<TelemetryRequestContext>>(|| None) = Some(req_ctx);
    }
    {
        let mut c = ctx_for(&mut state, &conn);
        hook.on_response_end(&mut c);
    }

    let mut seen = false;
    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let net_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM net_events WHERE credential_ref = ?1",
                [&credential_ref],
                |row| row.get(0),
            )
            .unwrap();
        let captured_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM substitution_events WHERE substitution_ref = ?1 AND outcome = 'captured'",
                [&credential_ref],
                |row| row.get(0),
            )
            .unwrap();
        let brokered_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM substitution_events WHERE substitution_ref = ?1 AND outcome = 'brokered'",
                [&credential_ref],
                |row| row.get(0),
            )
            .unwrap();
        if net_count == 1 && captured_count == 1 && brokered_count == 1 {
            seen = true;
            break;
        }
    }

    assert!(
        seen,
        "expected net and substitution rows with shared credential_ref"
    );
    let db_bytes = std::fs::read(&db_path).unwrap();
    assert!(
        !String::from_utf8_lossy(&db_bytes).contains(raw),
        "raw credential leaked into session db"
    );
}

#[tokio::test]
async fn hook_writes_security_rule_ledger_for_matching_http_event() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let rules_profile = SecurityRuleProfile::parse_toml(
        r#"
[profiles.rules.anthropic_http_seen]
name = "anthropic_http_seen"
action = "allow"
detection_level = "informational"
match = 'http.host == "api.anthropic.com" && http.path == "/v1/messages"'
"#,
    )
    .expect("rules parse");
    let rules = SecurityRuleSet::compile_profile(&rules_profile, SecurityRuleSource::User)
        .expect("rules compile");
    let db = Arc::new(DbWriter::open(&db_path, 64).expect("test db"));
    let deps = Arc::new(TelemetryDeps {
        db: Arc::clone(&db),
        pricing: Arc::new(PricingTable::load()),
        trace_state: Arc::new(Mutex::new(TraceState::new())),
        security_rules: Arc::new(std::sync::RwLock::new(Arc::new(rules))),
        plugin_policy: Arc::new(std::sync::RwLock::new(BTreeMap::new())),
    });
    let hook = TelemetryHook::new(deps);

    let mut state = HookState::default();
    let conn = any_conn();
    {
        let mut c = ctx_for(&mut state, &conn);
        *c.state::<Option<TelemetryRequestContext>>(|| None) = Some(anthropic_req_ctx());
    }
    {
        let mut c = ctx_for(&mut state, &conn);
        hook.on_response_end(&mut c);
    }

    let mut seen = false;
    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let joined: Option<(String, String, String)> = conn
            .query_row(
                "SELECT net_events.event_id, security_rule_events.rule_id, security_rule_events.detection_level
                 FROM net_events
                 JOIN security_rule_events ON security_rule_events.event_id = net_events.event_id
                 WHERE net_events.domain = 'api.anthropic.com'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .ok();
        let Some((event_id, rule_id, detection_level)) = joined else {
            continue;
        };
        assert_eq!(event_id.len(), 12);
        assert!(event_id
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)));
        assert_eq!(rule_id, "profiles.rules.anthropic_http_seen");
        assert_eq!(detection_level, "informational");
        seen = true;
        break;
    }

    assert!(
        seen,
        "expected HTTP telemetry to write a joined rule ledger row"
    );
}

#[tokio::test]
async fn hook_writes_security_rule_ledger_for_matching_model_event() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let rules_profile = SecurityRuleProfile::parse_toml(
        r#"
[profiles.rules.anthropic_model_seen]
name = "anthropic_model_seen"
action = "allow"
detection_level = "informational"
match = 'model.provider == "anthropic" && model.name == "claude-test"'
"#,
    )
    .expect("rules parse");
    let rules = SecurityRuleSet::compile_profile(&rules_profile, SecurityRuleSource::User)
        .expect("rules compile");
    let db = Arc::new(DbWriter::open(&db_path, 64).expect("test db"));
    let deps = Arc::new(TelemetryDeps {
        db: Arc::clone(&db),
        pricing: Arc::new(PricingTable::load()),
        trace_state: Arc::new(Mutex::new(TraceState::new())),
        security_rules: Arc::new(std::sync::RwLock::new(Arc::new(rules))),
        plugin_policy: Arc::new(std::sync::RwLock::new(BTreeMap::new())),
    });
    let hook = TelemetryHook::new(deps);

    let mut state = HookState::default();
    let conn = any_conn();
    {
        let mut c = ctx_for(&mut state, &conn);
        *c.state::<Option<TelemetryRequestContext>>(|| None) = Some(anthropic_req_ctx());
    }
    {
        let mut c = ctx_for(&mut state, &conn);
        hook.on_response_end(&mut c);
    }

    let mut seen = false;
    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let joined: Option<(String, String, String)> = conn
            .query_row(
                "SELECT model_calls.event_id, security_rule_events.rule_id, security_rule_events.detection_level
                 FROM model_calls
                 JOIN security_rule_events ON security_rule_events.event_id = model_calls.event_id
                 WHERE model_calls.provider = 'anthropic'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .ok();
        let Some((event_id, rule_id, detection_level)) = joined else {
            continue;
        };
        assert_eq!(event_id.len(), 12);
        assert!(event_id
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)));
        assert_eq!(rule_id, "profiles.rules.anthropic_model_seen");
        assert_eq!(detection_level, "informational");
        seen = true;
        break;
    }

    assert!(
        seen,
        "expected model telemetry to write a joined rule ledger row"
    );
}

#[tokio::test]
async fn hook_writes_injected_substitution_event_for_broker_ref_replay() {
    let _lock = crate::credential_broker::TEST_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let capsem_home = dir.path().join("capsem-home");
    let test_store = dir.path().join("credential-store.json");
    let _guard = EnvGuard::install(&capsem_home, dir.path(), &test_store);

    let db = Arc::new(DbWriter::open(&db_path, 64).expect("test db"));
    let deps = Arc::new(TelemetryDeps {
        db: Arc::clone(&db),
        pricing: Arc::new(PricingTable::load()),
        trace_state: Arc::new(Mutex::new(TraceState::new())),
        security_rules: empty_security_rules(),
        plugin_policy: Arc::new(std::sync::RwLock::new(BTreeMap::new())),
    });
    let hook = TelemetryHook::new(deps);
    let raw = "sk-ant-replayed-hook-test";
    let credential_ref = credential_reference("anthropic", raw);
    let mut req_ctx = anthropic_req_ctx();
    req_ctx.credential_ref = Some(credential_ref.clone());
    req_ctx.request_headers = Some(format!("authorization: Bearer {credential_ref}"));
    req_ctx.credential_injections = vec![CredentialInjection {
        provider: Some(CredentialProvider::Anthropic),
        credential_ref: credential_ref.clone(),
        source: "http.header.authorization".to_string(),
        event_type: Some("http.request".to_string()),
        confidence: 1.0,
        trace_id: Some("trace-injected-hook".to_string()),
        context_json: Some(r#"{"domain":"api.anthropic.com"}"#.to_string()),
    }];

    let mut state = HookState::default();
    let conn = any_conn();
    {
        let mut c = ctx_for(&mut state, &conn);
        *c.state::<Option<TelemetryRequestContext>>(|| None) = Some(req_ctx);
    }
    {
        let mut c = ctx_for(&mut state, &conn);
        hook.on_response_end(&mut c);
    }

    let mut seen = false;
    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let injected_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM substitution_events WHERE substitution_ref = ?1 AND outcome = 'injected'",
                [&credential_ref],
                |row| row.get(0),
            )
            .unwrap();
        let net_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM net_events WHERE credential_ref = ?1",
                [&credential_ref],
                |row| row.get(0),
            )
            .unwrap();
        if injected_count == 1 && net_count == 1 {
            seen = true;
            break;
        }
    }

    assert!(
        seen,
        "expected injected substitution row with shared net credential_ref"
    );
    let db_bytes = std::fs::read(&db_path).unwrap();
    assert!(
        !String::from_utf8_lossy(&db_bytes).contains(raw),
        "raw credential leaked into session db"
    );
}

#[tokio::test]
async fn hook_detects_response_body_token_exchange_and_redacts_preview() {
    let _lock = crate::credential_broker::TEST_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let capsem_home = dir.path().join("capsem-home");
    let test_store = dir.path().join("credential-store.json");
    let _guard = EnvGuard::install(&capsem_home, dir.path(), &test_store);

    let db = Arc::new(DbWriter::open(&db_path, 64).expect("test db"));
    let deps = Arc::new(TelemetryDeps {
        db: Arc::clone(&db),
        pricing: Arc::new(PricingTable::load()),
        trace_state: Arc::new(Mutex::new(TraceState::new())),
        security_rules: empty_security_rules(),
        plugin_policy: Arc::new(std::sync::RwLock::new(BTreeMap::new())),
    });
    let hook = TelemetryHook::new(deps);
    let raw = "github_pat_exchange_secret";

    let mut req_ctx = anthropic_req_ctx();
    req_ctx.domain = "api.github.com".to_string();
    req_ctx.ai_provider = None;
    req_ctx.path = "/login/oauth/access_token".to_string();
    req_ctx.request_headers = Some("host: api.github.com".to_string());
    req_ctx.response_headers = Some("content-type: application/json".to_string());

    let mut state = HookState::default();
    let conn = ConnMeta {
        domain: "api.github.com".to_string(),
        port: 443,
        process_name: None,
        ..Default::default()
    };
    {
        let mut c = ctx_for(&mut state, &conn);
        *c.state::<Option<TelemetryRequestContext>>(|| None) = Some(req_ctx);
        *c.state::<TelemetryResponseStats>(TelemetryResponseStats::default) =
            TelemetryResponseStats {
                bytes: raw.len() as u64,
                preview: format!(r#"{{"access_token":"{raw}","token_type":"bearer"}}"#)
                    .into_bytes(),
                max_preview: 4096,
            };
    }
    {
        let mut c = ctx_for(&mut state, &conn);
        hook.on_response_end(&mut c);
    }

    let mut seen = false;
    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let row: Option<(String, String)> = conn
            .query_row(
                "SELECT credential_ref, response_body_preview FROM net_events WHERE domain = 'api.github.com'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();
        let Some((credential_ref, preview)) = row else {
            continue;
        };
        let sub_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM substitution_events WHERE substitution_ref = ?1 AND source = 'http.body.response.$.access_token'",
                [&credential_ref],
                |row| row.get(0),
            )
            .unwrap();
        assert!(credential_ref.starts_with("credential:blake3:"));
        assert!(preview.contains("credential:blake3:"));
        assert!(!preview.contains(raw));
        if sub_count == 1 {
            seen = true;
            break;
        }
    }

    assert!(
        seen,
        "expected token exchange response to be brokered and redacted"
    );
}
