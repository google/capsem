use capsem_security_engine::detection_ir::{
    compile_detection_ir_to_cel_detection_rules, evaluate_detection_ir,
    evaluate_detection_ir_security_event, parse_detection_ir_v1_json, DetectionIRMatcherV1,
    DetectionIRV1, DetectionOperator, EventFamily, SecurityEventV1,
};
use capsem_security_engine::{
    AiAttributionScope, AiOriginKind, CelDetectionEvaluator, DetectionEvaluator, Enforceability,
    HttpBodySecuritySubject, HttpSecuritySubject, RedactionState, SecurityEvent,
    SecurityEventCommon, SecurityEventType, SourceEngine,
};
use criterion::{black_box, criterion_group, criterion_main, Criterion};

const GOOGLE_SECRET_IR_JSON: &str =
    include_str!("../../../data/detection/ir/google-secret-egress.json");

fn google_secret_ir() -> DetectionIRV1 {
    parse_detection_ir_v1_json(GOOGLE_SECRET_IR_JSON).unwrap()
}

fn hundred_rule_ir() -> DetectionIRV1 {
    let mut ir = google_secret_ir();
    let template = ir.rules[0].clone();
    ir.rules = (0..100)
        .map(|index| {
            let mut rule = template.clone();
            rule.id = format!("detect-google-secret-{index:03}");
            rule.source_id = rule.id.clone();
            rule.sigma_id = Some(format!("sigma-google-secret-{index:03}"));
            rule
        })
        .collect();
    ir
}

fn family_matchers() -> Vec<(EventFamily, &'static str, serde_json::Value)> {
    vec![
        (
            EventFamily::Dns,
            "dns.request.qname",
            serde_json::json!("google.example.test"),
        ),
        (
            EventFamily::Http,
            "http.request.host",
            serde_json::json!("api.example.test"),
        ),
        (
            EventFamily::Mcp,
            "mcp.request.tool_name",
            serde_json::json!("read_file"),
        ),
        (
            EventFamily::Model,
            "model.request.provider",
            serde_json::json!("google_gemini"),
        ),
        (
            EventFamily::File,
            "file.activity.path_class",
            serde_json::json!("workspace"),
        ),
        (
            EventFamily::Process,
            "process.activity.command_class",
            serde_json::json!("shell"),
        ),
        (
            EventFamily::Credential,
            "credential.activity.credential_id",
            serde_json::json!("api-token"),
        ),
        (
            EventFamily::Vm,
            "vm.activity.operation",
            serde_json::json!("start"),
        ),
        (
            EventFamily::Profile,
            "profile.activity.profile_id",
            serde_json::json!("coding"),
        ),
        (
            EventFamily::Conversation,
            "conversation.activity.conversation_id",
            serde_json::json!("conv-1"),
        ),
        (
            EventFamily::Snapshot,
            "snapshot.activity.snapshot_id",
            serde_json::json!("snap-1"),
        ),
    ]
}

fn mixed_family_ir(rule_count: usize) -> DetectionIRV1 {
    let mut ir = google_secret_ir();
    let template = ir.rules[0].clone();
    let matchers = family_matchers();
    ir.rules = (0..rule_count)
        .map(|index| {
            let (family, field_path, value) = &matchers[index % matchers.len()];
            let mut rule = template.clone();
            rule.id = format!("detect-{family:?}-{index:03}").to_lowercase();
            rule.source_id = rule.id.clone();
            rule.sigma_id = Some(format!("sigma-{family:?}-{index:03}").to_lowercase());
            rule.event_family = *family;
            rule.matchers = vec![DetectionIRMatcherV1 {
                field_path: (*field_path).into(),
                operator: DetectionOperator::EqualsAny,
                values: vec![value.clone()],
                sigma_field: (*field_path).into(),
            }];
            rule
        })
        .collect();
    ir
}

fn indexed_model_tool_ir() -> DetectionIRV1 {
    let mut ir = google_secret_ir();
    let rule = &mut ir.rules[0];
    rule.id = "detect-indexed-model-tool".into();
    rule.source_id = rule.id.clone();
    rule.sigma_id = Some("sigma-indexed-model-tool".into());
    rule.event_family = EventFamily::Model;
    rule.matchers = vec![
        DetectionIRMatcherV1 {
            field_path: "model.request.tool_calls[0].name".into(),
            operator: DetectionOperator::EqualsAny,
            values: vec![serde_json::json!("filesystem.read_file")],
            sigma_field: "tool_name".into(),
        },
        DetectionIRMatcherV1 {
            field_path: "model.response.tool_results[0].returned_to_model".into(),
            operator: DetectionOperator::EqualsAny,
            values: vec![serde_json::json!(true)],
            sigma_field: "returned_to_model".into(),
        },
    ];
    ir
}

fn legacy_http_match_event() -> SecurityEventV1 {
    serde_json::from_value(serde_json::json!({
        "event_id": "evt-bench-detection-ir-http",
        "event_family": "http",
        "event_type": "http.request",
        "subject": {
            "request": {
                "host": "googleapis.com",
                "body": {
                    "text": "token=secret"
                }
            }
        }
    }))
    .unwrap()
}

fn common_for(event_id: &str, event_type: &str) -> SecurityEventCommon {
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
        trace_id: Some("trace-detection-ir-bench".into()),
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
        event_type: SecurityEventType::parse(event_type).unwrap(),
        redaction_state: RedactionState::Raw,
    }
}

fn canonical_http_match_event() -> SecurityEvent {
    SecurityEvent::http(
        common_for("evt-bench-canonical-detection-ir-http", "http.request"),
        HttpSecuritySubject {
            method: "POST".into(),
            scheme: Some("https".into()),
            host: "googleapis.com".into(),
            port: Some(443),
            path: Some("/upload".into()),
            query: None,
            url: Some("https://googleapis.com/upload".into()),
            path_class: "external".into(),
            request_bytes: 128,
            request_headers: Default::default(),
            request_body: Some(HttpBodySecuritySubject::text("token=secret")),
            response_status: None,
            response_headers: Default::default(),
            response_bytes: None,
            response_body: None,
        },
    )
}

fn bench_detection_ir_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("security_engine_detection_ir_parse");

    group.bench_function("parse_validate_google_secret_fixture", |b| {
        b.iter(|| black_box(parse_detection_ir_v1_json(black_box(GOOGLE_SECRET_IR_JSON))).unwrap());
    });

    group.finish();
}

fn bench_detection_ir_lowering(c: &mut Criterion) {
    let single_rule = google_secret_ir();
    let hundred_rules = hundred_rule_ir();
    let every_family = mixed_family_ir(family_matchers().len());
    let hundred_mixed_family_rules = mixed_family_ir(100);
    let indexed_model_tool = indexed_model_tool_ir();
    let mut group = c.benchmark_group("security_engine_detection_ir_lowering");

    group.bench_function("lower_google_secret_fixture_to_cel_rules", |b| {
        b.iter(|| {
            let rules = compile_detection_ir_to_cel_detection_rules(black_box(&single_rule))
                .expect("fixture should lower");
            black_box(rules.len())
        });
    });

    group.bench_function("lower_100_http_rules_to_cel_rules", |b| {
        b.iter(|| {
            let rules = compile_detection_ir_to_cel_detection_rules(black_box(&hundred_rules))
                .expect("fixture should lower");
            black_box(rules.len())
        });
    });

    group.bench_function("lower_and_compile_100_http_rules", |b| {
        b.iter(|| {
            let rules = compile_detection_ir_to_cel_detection_rules(black_box(&hundred_rules))
                .expect("fixture should lower");
            let evaluator = CelDetectionEvaluator::compile(black_box(rules)).unwrap();
            black_box(evaluator)
        });
    });

    group.bench_function("lower_every_event_family_to_cel_rules", |b| {
        b.iter(|| {
            let rules = compile_detection_ir_to_cel_detection_rules(black_box(&every_family))
                .expect("fixture should lower");
            black_box(rules.len())
        });
    });

    group.bench_function("lower_indexed_model_tool_paths_to_cel_rules", |b| {
        b.iter(|| {
            let rules = compile_detection_ir_to_cel_detection_rules(black_box(&indexed_model_tool))
                .expect("fixture should lower");
            black_box(rules.len())
        });
    });

    group.bench_function("lower_100_mixed_family_rules_to_cel_rules", |b| {
        b.iter(|| {
            let rules =
                compile_detection_ir_to_cel_detection_rules(black_box(&hundred_mixed_family_rules))
                    .expect("fixture should lower");
            black_box(rules.len())
        });
    });

    group.bench_function("lower_and_compile_100_mixed_family_rules", |b| {
        b.iter(|| {
            let rules =
                compile_detection_ir_to_cel_detection_rules(black_box(&hundred_mixed_family_rules))
                    .expect("fixture should lower");
            let evaluator = CelDetectionEvaluator::compile(black_box(rules)).unwrap();
            black_box(evaluator)
        });
    });

    group.finish();
}

fn bench_detection_ir_matching(c: &mut Criterion) {
    let ir = google_secret_ir();
    let legacy_event = legacy_http_match_event();
    let canonical_event = canonical_http_match_event();
    let cel_rules = compile_detection_ir_to_cel_detection_rules(&ir).expect("fixture should lower");
    let mut cel_evaluator = CelDetectionEvaluator::compile(cel_rules).unwrap();
    let mut group = c.benchmark_group("security_engine_detection_ir_matching");

    group.bench_function("direct_match_legacy_detection_event", |b| {
        b.iter(|| {
            black_box(evaluate_detection_ir(
                black_box(&ir),
                black_box(&legacy_event),
            ))
        });
    });

    group.bench_function("direct_match_canonical_security_event", |b| {
        b.iter(|| {
            black_box(evaluate_detection_ir_security_event(
                black_box(&ir),
                black_box(&canonical_event),
            ))
        });
    });

    group.bench_function("lowered_cel_match_canonical_security_event", |b| {
        b.iter(|| black_box(cel_evaluator.evaluate(black_box(&canonical_event)).unwrap()));
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_detection_ir_parse,
    bench_detection_ir_lowering,
    bench_detection_ir_matching
);
criterion_main!(benches);
