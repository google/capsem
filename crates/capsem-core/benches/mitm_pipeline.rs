//! MITM pipeline microbenchmarks for the security spine.
//!
//! These measure the host-side callback work that happens once MITM traffic has
//! already been parsed into request/response context: canonical security-event
//! construction plus `SecurityEngine` evaluation. Provider body parsing has its
//! own focused benchmark in `provider_model_parser`.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use capsem_core::net::ai_traffic::pricing::PricingTable;
use capsem_core::net::ai_traffic::provider::ProviderKind;
use capsem_core::net::ai_traffic::TraceState;
use capsem_core::net::mitm_proxy::body::BodyStats;
use capsem_core::net::mitm_proxy::telemetry_hook::{
    build_http_response_security_event, build_http_security_event,
    build_model_request_security_event, build_model_response_security_event,
    new_http_event_id_seed, parse_llm_events_from_response_body, TelemetryDeps,
    TelemetryIdentityContext, TelemetryRequestContext, TelemetryResponseStats,
};
use capsem_logger::{DbWriter, Decision};
use capsem_security_engine::{
    CelEnforcementEvaluator, CelEnforcementRule, SecurityDecisionAction, SecurityEngine,
};
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use metrics::counter;

const OPENAI_REQUEST: &[u8] = br#"{
  "model": "gpt-bench",
  "stream": true,
  "messages": [
    { "role": "system", "content": "benchmark system prompt" },
    { "role": "user", "content": "say hello" }
  ],
  "tools": [
    {
      "type": "function",
      "function": {
        "name": "mcp.filesystem.read_file",
        "parameters": { "type": "object" }
      }
    }
  ]
}"#;

const OPENAI_RESPONSE: &[u8] = b"data: {\"id\":\"chatcmpl-bench\",\"model\":\"gpt-bench\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hello from model\"},\"finish_reason\":null}]}\n\n\
data: {\"id\":\"chatcmpl-bench\",\"model\":\"gpt-bench\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n\
data: [DONE]\n\n";

fn rule(id: &str, condition: &str) -> CelEnforcementRule {
    CelEnforcementRule {
        id: id.into(),
        pack_id: Some("bench.mitm".into()),
        condition: condition.into(),
        decision: SecurityDecisionAction::Block,
        reason: Some("benchmark block".into()),
        mutations: Vec::new(),
    }
}

fn security_engine(rules: Vec<CelEnforcementRule>) -> SecurityEngine {
    let mut engine = SecurityEngine::default();
    engine.set_enforcement(Box::new(CelEnforcementEvaluator::compile(rules).unwrap()));
    engine
}

fn http_security_engine() -> SecurityEngine {
    security_engine(vec![
        rule(
            "bench-http-request-callback",
            "common.event_type == 'http.request' && http.request.host == 'blocked.example'",
        ),
        rule(
            "bench-http-response-callback",
            "common.event_type == 'http.response' && http.response.body.text.contains('blocked-response')",
        ),
    ])
}

fn model_security_engine() -> SecurityEngine {
    security_engine(vec![
        rule(
            "bench-model-request-callback",
            "common.event_type == 'model.request' && model.request.model == 'blocked-model'",
        ),
        rule(
            "bench-model-response-callback",
            "common.event_type == 'model.response' && model.response.body.text.contains('blocked-model')",
        ),
    ])
}

fn request_body_stats(body: &[u8]) -> Arc<Mutex<BodyStats>> {
    Arc::new(Mutex::new(BodyStats {
        bytes: body.len() as u64,
        preview: body.to_vec(),
        max_preview: 4096,
    }))
}

fn request_context(
    ai_provider: Option<ProviderKind>,
    status_code: Option<u16>,
) -> TelemetryRequestContext {
    TelemetryRequestContext {
        event_id_seed: new_http_event_id_seed(),
        domain: "api.openai.com".into(),
        process_name: Some("bench-client".into()),
        ai_provider,
        method: "POST".into(),
        path: "/v1/chat/completions".into(),
        query: Some("source=criterion".into()),
        status_code,
        decision: Decision::Allowed,
        matched_rule: None,
        request_headers: Some(
            "host: api.openai.com\r\ncontent-type: application/json\r\nauthorization: Bearer bench\r\n"
                .into(),
        ),
        response_headers: Some("content-type: text/event-stream\r\n".into()),
        start_time: Instant::now(),
        request_body_stats: request_body_stats(OPENAI_REQUEST),
        max_response_preview: 4096,
        port: 443,
        conn_type: "https-mitm",
        identity: TelemetryIdentityContext {
            vm_id: Some("bench-vm".into()),
            session_id: Some("bench-session".into()),
            profile_id: Some("coding".into()),
            profile_revision: Some("2026.0603.1".into()),
            user_id: Some("bench-user".into()),
        },
        policy_mode: Some("runtime".into()),
        policy_action: None,
        policy_rule: None,
        policy_reason: None,
        runtime_security_results: Vec::new(),
    }
}

fn telemetry_deps() -> TelemetryDeps {
    TelemetryDeps {
        db: Arc::new(DbWriter::open_in_memory(16).unwrap()),
        pricing: Arc::new(PricingTable::load()),
        trace_state: Arc::new(Mutex::new(TraceState::new())),
    }
}

fn bench_describe(c: &mut Criterion) {
    c.bench_function("metrics_describe_all", |b| {
        b.iter(|| {
            capsem_core::net::mitm_proxy::metrics::describe_all();
        });
    });
}

fn bench_counter_emit(c: &mut Criterion) {
    c.bench_function("counter_emit_no_recorder", |b| {
        b.iter(|| {
            counter!(black_box("mitm.requests_total"), "decision" => "allow").increment(1);
        });
    });
}

fn bench_http_security_callbacks(c: &mut Criterion) {
    let mut engine = http_security_engine();
    let request_ctx = request_context(None, None);
    let response_ctx = request_context(None, Some(200));
    let timestamp_unix_ms = 1_789_003_001;
    let trace_id = Some("trace-bench-mitm-http".to_string());
    let request_event = build_http_security_event(
        &request_ctx,
        timestamp_unix_ms,
        trace_id.clone(),
        None,
        None,
    );
    let response_event = build_http_response_security_event(
        &response_ctx,
        timestamp_unix_ms,
        trace_id.clone(),
        Some(OPENAI_RESPONSE.len() as u64),
        Some("hello from model".into()),
    );

    let mut group = c.benchmark_group("mitm_security_callback_http");
    group.bench_function("request_build_event_then_evaluate", |b| {
        b.iter(|| {
            let event = build_http_security_event(
                black_box(&request_ctx),
                black_box(timestamp_unix_ms),
                black_box(trace_id.clone()),
                None,
                None,
            );
            let result = engine.evaluate(black_box(event)).unwrap();
            black_box(result.action)
        });
    });
    group.bench_function("request_evaluate_prebuilt_event", |b| {
        b.iter(|| {
            let result = engine.evaluate(black_box(request_event.clone())).unwrap();
            black_box(result.action)
        });
    });
    group.bench_function("response_build_event_then_evaluate", |b| {
        b.iter(|| {
            let event = build_http_response_security_event(
                black_box(&response_ctx),
                black_box(timestamp_unix_ms),
                black_box(trace_id.clone()),
                Some(OPENAI_RESPONSE.len() as u64),
                Some("hello from model".into()),
            );
            let result = engine.evaluate(black_box(event)).unwrap();
            black_box(result.action)
        });
    });
    group.bench_function("response_evaluate_prebuilt_event", |b| {
        b.iter(|| {
            let result = engine.evaluate(black_box(response_event.clone())).unwrap();
            black_box(result.action)
        });
    });
    group.finish();
}

fn bench_model_security_callbacks(c: &mut Criterion) {
    let mut engine = model_security_engine();
    let deps = telemetry_deps();
    let request_ctx = request_context(Some(ProviderKind::OpenAi), None);
    let response_ctx = request_context(Some(ProviderKind::OpenAi), Some(200));
    let timestamp_unix_ms = 1_789_003_001;
    let trace_id = Some("trace-bench-mitm-model".to_string());
    let response_stats = TelemetryResponseStats {
        bytes: OPENAI_RESPONSE.len() as u64,
        preview: OPENAI_RESPONSE.to_vec(),
        max_preview: 4096,
    };
    let llm_events = parse_llm_events_from_response_body(ProviderKind::OpenAi, OPENAI_RESPONSE);
    let request_event = build_model_request_security_event(
        &request_ctx,
        OPENAI_REQUEST,
        timestamp_unix_ms,
        trace_id.clone(),
    )
    .unwrap();
    let response_event = build_model_response_security_event(
        &deps,
        &response_ctx,
        &response_stats,
        &llm_events,
        timestamp_unix_ms,
        trace_id.clone(),
    )
    .unwrap();

    let mut group = c.benchmark_group("mitm_security_callback_model");
    group.bench_function("request_build_event_then_evaluate", |b| {
        b.iter(|| {
            let event = build_model_request_security_event(
                black_box(&request_ctx),
                black_box(OPENAI_REQUEST),
                black_box(timestamp_unix_ms),
                black_box(trace_id.clone()),
            )
            .unwrap();
            let result = engine.evaluate(black_box(event)).unwrap();
            black_box(result.action)
        });
    });
    group.bench_function("request_evaluate_prebuilt_event", |b| {
        b.iter(|| {
            let result = engine.evaluate(black_box(request_event.clone())).unwrap();
            black_box(result.action)
        });
    });
    group.bench_function("response_build_event_then_evaluate", |b| {
        b.iter(|| {
            let event = build_model_response_security_event(
                black_box(&deps),
                black_box(&response_ctx),
                black_box(&response_stats),
                black_box(&llm_events),
                black_box(timestamp_unix_ms),
                black_box(trace_id.clone()),
            )
            .unwrap();
            let result = engine.evaluate(black_box(event)).unwrap();
            black_box(result.action)
        });
    });
    group.bench_function("response_evaluate_prebuilt_event", |b| {
        b.iter(|| {
            let result = engine.evaluate(black_box(response_event.clone())).unwrap();
            black_box(result.action)
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_describe,
    bench_counter_emit,
    bench_http_security_callbacks,
    bench_model_security_callbacks
);
criterion_main!(benches);
