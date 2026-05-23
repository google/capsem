use std::collections::BTreeMap;

use capsem_security_engine::{
    dedupe_backtest_matches, policy_context_from_event, AiAttributionScope, AiOriginKind,
    BacktestEventRef, BacktestMatchRow, BacktestOutcome, CelDetectionEvaluator, CelDetectionRule,
    CelEnforcementEvaluator, CelEnforcementRule, Confidence, DetectionEvaluator, Enforceability,
    EnforcementEvaluator, HttpBodySecuritySubject, HttpSecuritySubject, MatchedField,
    RedactionState, RuleOrigin, RuleRegistryError, RuleScope, RuntimeRuleDefinition,
    RuntimeRuleMetadata, RuntimeRuleRecord, RuntimeRuleRegistry, SecurityDecisionAction,
    SecurityEvent, SecurityEventCommon, SecurityEventSubject, Severity, SourceEngine,
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

fn detection_rule(id: impl Into<String>, condition: impl Into<String>) -> CelDetectionRule {
    CelDetectionRule {
        id: id.into(),
        pack_id: "bench.detection".into(),
        sigma_id: Some("sigma-bench".into()),
        title: "Benchmark detection".into(),
        condition: condition.into(),
        severity: Severity::Medium,
        confidence: Confidence::High,
        tags: vec!["benchmark".into(), "http".into()],
    }
}

fn registry_enforcement_record(
    id: impl Into<String>,
    condition: impl Into<String>,
) -> RuntimeRuleRecord {
    RuntimeRuleRecord {
        metadata: RuntimeRuleMetadata {
            id: id.into(),
            pack_id: Some("bench.registry".into()),
            scope: RuleScope::Runtime,
            origin: RuleOrigin::Runtime,
            priority: 100,
        },
        definition: RuntimeRuleDefinition::Enforcement {
            decision: SecurityDecisionAction::Block,
            reason: Some("benchmark registry update".into()),
        },
        source: condition.into(),
        enabled: true,
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

fn detection_evaluator(condition: &str) -> CelDetectionEvaluator {
    CelDetectionEvaluator::compile(vec![detection_rule("bench-detection", condition)]).unwrap()
}

fn last_match_detection_evaluator(rule_count: usize) -> CelDetectionEvaluator {
    let mut rules = Vec::with_capacity(rule_count);
    for index in 0..rule_count.saturating_sub(1) {
        rules.push(detection_rule(
            format!("bench-detect-no-match-{index}"),
            "false",
        ));
    }
    rules.push(detection_rule(
        "bench-detect-last-match",
        CANONICAL_HTTP_POLICY,
    ));
    CelDetectionEvaluator::compile(rules).unwrap()
}

fn backtest_rows(row_count: usize, unique_signatures: usize) -> Vec<BacktestMatchRow> {
    (0..row_count)
        .map(|index| BacktestMatchRow {
            event_ref: BacktestEventRef {
                corpus: "criterion".into(),
                session_id: Some("bench-session".into()),
                event_id: format!("evt-backtest-{index}"),
                sequence_no: Some(index as u64),
                timestamp_unix_ms: 1_789_003_001 + index as u64,
            },
            rule_id: "bench-detect".into(),
            pack_id: "bench.pack".into(),
            evidence_signature: format!("evidence-{}", index % unique_signatures),
            matched_fields: vec![MatchedField {
                path: "http.request.host".into(),
                value: serde_json::json!("googleapis.com"),
            }],
            outcome: BacktestOutcome::Matched,
        })
        .collect()
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

fn bench_detection(c: &mut Criterion) {
    let event = http_event();
    let mut group = c.benchmark_group("security_engine_detection_evaluate");

    let mut single_rule = detection_evaluator(CANONICAL_HTTP_POLICY);
    group.bench_function("canonical_http_policy_single_rule", |b| {
        b.iter(|| {
            let findings = single_rule.evaluate(black_box(&event)).unwrap();
            black_box(findings.len())
        });
    });

    let mut hundred_rules = last_match_detection_evaluator(100);
    group.bench_function("canonical_http_policy_last_match_100_rules", |b| {
        b.iter(|| {
            let findings = hundred_rules.evaluate(black_box(&event)).unwrap();
            black_box(findings.len())
        });
    });

    group.finish();
}

fn bench_backtest_dedupe(c: &mut Criterion) {
    let rows_100 = backtest_rows(100, 100);
    let rows_1000 = backtest_rows(1_000, 100);
    let mut group = c.benchmark_group("security_engine_backtest_dedupe");

    group.bench_function("dedupe_100_unique_limit_100", |b| {
        b.iter(|| {
            let result = dedupe_backtest_matches(black_box(rows_100.clone()), 100);
            black_box(result.rows.len())
        });
    });

    group.bench_function("dedupe_1000_rows_100_unique_limit_100", |b| {
        b.iter(|| {
            let result = dedupe_backtest_matches(black_box(rows_1000.clone()), 100);
            black_box(result.rows.len())
        });
    });

    group.finish();
}

fn bench_runtime_registry(c: &mut Criterion) {
    let mut group = c.benchmark_group("security_engine_runtime_registry");

    group.bench_function("add_or_update_single_rule", |b| {
        let mut generation = 0_u64;
        b.iter(|| {
            generation += 1;
            let mut registry = RuntimeRuleRegistry::default();
            registry
                .add_or_update(
                    registry_enforcement_record(
                        format!("bench-runtime-{generation}"),
                        CANONICAL_HTTP_POLICY,
                    ),
                    |_| Ok::<_, RuleRegistryError>("compiled-plan".into()),
                )
                .unwrap();
            black_box(registry.list().len())
        });
    });

    group.bench_function("enabled_enforcement_rules_100_rules", |b| {
        let mut registry = RuntimeRuleRegistry::default();
        for index in 0..100 {
            registry
                .add_or_update(
                    registry_enforcement_record(format!("bench-runtime-{index:03}"), "false"),
                    |_| Ok::<_, RuleRegistryError>("compiled-plan".into()),
                )
                .unwrap();
        }
        b.iter(|| black_box(registry.enabled_enforcement_rules().len()));
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
    bench_detection,
    bench_backtest_dedupe,
    bench_runtime_registry,
    bench_materialization,
    bench_native_lookup
);
criterion_main!(benches);
