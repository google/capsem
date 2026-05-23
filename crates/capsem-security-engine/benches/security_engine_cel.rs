use std::collections::BTreeMap;

use capsem_security_engine::{
    policy_context_from_event, AiAttributionScope, AiOriginKind, CelEnforcementEvaluator,
    CelEnforcementRule, Enforceability, EnforcementEvaluator, HttpBodySecuritySubject,
    HttpSecuritySubject, RedactionState, SecurityDecisionAction, SecurityEvent,
    SecurityEventCommon, SecurityEventSubject, SourceEngine,
};
use criterion::{black_box, criterion_group, criterion_main, Criterion};

const HOST_CONTAINS_GOOGLE: &str = "http.request.host.contains('google')";
const URL_CONTAINS_GOOGLE: &str = "http.request.url.contains('google')";
const PATH_STARTS_ADMIN: &str = "http.request.path.startsWith('/admin')";
const HEADER_AUTH_EXISTS: &str = "http.request.header('authorization').exists()";
const BODY_CONTAINS_SECRET: &str = "http.request.body.text.contains('secret')";
const CANONICAL_HTTP_POLICY: &str = "\
    http.request.host.contains('google') \
    && http.request.url.contains('google') \
    && http.request.path.startsWith('/admin') \
    && http.request.header('authorization').exists() \
    && http.request.body.text.contains('secret')";

fn common(event_id: &str) -> SecurityEventCommon {
    SecurityEventCommon {
        event_id: event_id.to_owned(),
        parent_event_id: None,
        stream_id: None,
        activity_id: None,
        sequence_no: Some(1),
        source_engine: SourceEngine::Network,
        attribution_scope: AiAttributionScope::Vm,
        origin_kind: AiOriginKind::GuestNetwork,
        accounting_owner: Some("vm:bench-vm".into()),
        enforceability: Enforceability::InlineBlockable,
        trace_id: Some("trace-bench".into()),
        span_id: None,
        timestamp_unix_ms: 1_789_003_001,
        vm_id: Some("bench-vm".into()),
        session_id: Some("bench-session".into()),
        profile_id: Some("coding".into()),
        profile_revision: Some("2026.0523.1".into()),
        profile_pack_ids: Vec::new(),
        enforcement_packs: Vec::new(),
        detection_packs: Vec::new(),
        user_id: Some("bench-user".into()),
        process_id: None,
        parent_process_id: None,
        exec_id: None,
        turn_id: None,
        message_id: None,
        tool_call_id: None,
        mcp_call_id: None,
        event_type: "http.request".into(),
        redaction_state: RedactionState::Raw,
    }
}

fn http_event() -> SecurityEvent {
    let mut request_headers = BTreeMap::new();
    request_headers.insert("Authorization".into(), vec!["Bearer bench-token".into()]);
    request_headers.insert("Content-Type".into(), vec!["text/plain".into()]);

    SecurityEvent::http(
        common("evt-bench-http-google-secret"),
        HttpSecuritySubject {
            method: "POST".into(),
            scheme: Some("https".into()),
            host: "googleapis.com".into(),
            port: Some(443),
            path: Some("/admin/upload".into()),
            query: Some("source=criterion".into()),
            url: Some("https://googleapis.com/admin/upload?source=criterion".into()),
            path_class: "admin".into(),
            request_bytes: 128,
            request_headers,
            request_body: Some(HttpBodySecuritySubject::text("token=secret")),
            response_status: Some(200),
            response_headers: BTreeMap::new(),
            response_bytes: Some(34),
            response_body: None,
        },
    )
}

fn rule(id: impl Into<String>, condition: impl Into<String>) -> CelEnforcementRule {
    CelEnforcementRule {
        id: id.into(),
        pack_id: Some("bench.enforcement".into()),
        condition: condition.into(),
        decision: SecurityDecisionAction::Block,
        reason: Some("benchmark match".into()),
    }
}

fn evaluator(condition: &str) -> CelEnforcementEvaluator {
    CelEnforcementEvaluator::compile(vec![rule("bench-rule", condition)]).unwrap()
}

fn last_match_evaluator(rule_count: usize) -> CelEnforcementEvaluator {
    let mut rules = Vec::with_capacity(rule_count);
    for index in 0..rule_count.saturating_sub(1) {
        rules.push(rule(format!("bench-no-match-{index}"), "false"));
    }
    rules.push(rule("bench-last-match", CANONICAL_HTTP_POLICY));
    CelEnforcementEvaluator::compile(rules).unwrap()
}

fn native_http_policy(event: &SecurityEvent) -> bool {
    let SecurityEventSubject::Http(subject) = &event.subject else {
        return false;
    };
    let has_authorization = subject
        .request_headers
        .keys()
        .any(|name| name.eq_ignore_ascii_case("authorization"));
    subject.host.contains("google")
        && subject
            .url
            .as_deref()
            .is_some_and(|url| url.contains("google"))
        && subject
            .path
            .as_deref()
            .is_some_and(|path| path.starts_with("/admin"))
        && has_authorization
        && subject
            .request_body
            .as_ref()
            .and_then(|body| body.text.as_deref())
            .is_some_and(|text| text.contains("secret"))
}

fn bench_compile(c: &mut Criterion) {
    let mut group = c.benchmark_group("security_engine_cel_compile");
    for (name, condition) in [
        ("host_contains_google", HOST_CONTAINS_GOOGLE),
        ("header_authorization_exists", HEADER_AUTH_EXISTS),
        ("canonical_http_policy", CANONICAL_HTTP_POLICY),
    ] {
        group.bench_function(name, |b| {
            b.iter(|| {
                black_box(CelEnforcementEvaluator::compile(vec![rule(
                    "bench-compile",
                    black_box(condition),
                )]))
                .unwrap();
            });
        });
    }
    group.finish();
}

fn bench_evaluate(c: &mut Criterion) {
    let event = http_event();
    let mut group = c.benchmark_group("security_engine_cel_evaluate");
    for (name, condition) in [
        ("host_contains_google", HOST_CONTAINS_GOOGLE),
        ("url_contains_google", URL_CONTAINS_GOOGLE),
        ("path_starts_admin", PATH_STARTS_ADMIN),
        ("header_authorization_exists", HEADER_AUTH_EXISTS),
        ("body_contains_secret", BODY_CONTAINS_SECRET),
        ("canonical_http_policy", CANONICAL_HTTP_POLICY),
    ] {
        let mut evaluator = evaluator(condition);
        group.bench_function(name, |b| {
            b.iter(|| {
                let decision = evaluator.evaluate(black_box(&event)).unwrap();
                black_box(decision.is_some())
            });
        });
    }

    let mut hundred_rules = last_match_evaluator(100);
    group.bench_function("canonical_http_policy_last_match_100_rules", |b| {
        b.iter(|| {
            let decision = hundred_rules.evaluate(black_box(&event)).unwrap();
            black_box(decision.is_some())
        });
    });
    group.finish();
}

fn bench_materialization(c: &mut Criterion) {
    let event = http_event();
    let mut group = c.benchmark_group("security_engine_policy_context");
    group.bench_function("project_security_event_to_policy_context", |b| {
        b.iter(|| black_box(policy_context_from_event(black_box(&event))));
    });
    group.bench_function("project_and_serialize_policy_context", |b| {
        b.iter(|| {
            let context = policy_context_from_event(black_box(&event));
            black_box(serde_json::to_value(context).unwrap())
        });
    });
    group.finish();
}

fn bench_native_lookup(c: &mut Criterion) {
    let event = http_event();
    c.bench_function("security_engine_native_lookup/canonical_http_policy", |b| {
        b.iter(|| black_box(native_http_policy(black_box(&event))));
    });
}

criterion_group!(
    benches,
    bench_compile,
    bench_evaluate,
    bench_materialization,
    bench_native_lookup
);
criterion_main!(benches);
