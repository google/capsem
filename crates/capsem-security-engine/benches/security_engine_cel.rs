use std::collections::BTreeMap;

use capsem_security_engine::{
    dedupe_backtest_matches, policy_context_from_event, run_detection_backtest, run_detection_hunt,
    run_enforcement_backtest, AiApiFamily, AiAttributionScope, AiContentKind, AiOriginKind,
    AiProvider, AiUsageEvidence, ArgumentsStatus, BacktestEventInput, BacktestEventRef,
    BacktestMatchRow, BacktestOutcome, CelDetectionEvaluator, CelDetectionRule,
    CelEnforcementEvaluator, CelEnforcementRule, Confidence, ConversationSecuritySubject,
    CredentialSecuritySubject, DetectionEvaluator, DnsSecuritySubject, Enforceability,
    EnforcementEvaluator, EventContext, EvidenceStatus, FileSecuritySubject,
    HttpBodySecuritySubject, HttpSecuritySubject, MatchedField, McpSecuritySubject,
    ModelInteractionEvidence, ModelRequestEvidence, ModelResponseEvidence, ModelSecuritySubject,
    ModelToolCallEvidence, ModelToolResultEvidence, ParseStatus, ProcessSecuritySubject,
    ProfileSecuritySubject, RedactionState, RuleOrigin, RuleRegistryError, RuleScope,
    RuntimeRuleDefinition, RuntimeRuleMetadata, RuntimeRuleRecord, RuntimeRuleRegistry,
    SecurityDecisionAction, SecurityEngine, SecurityEvent, SecurityEventCommon,
    SecurityEventSubject, SecurityEventType, Severity, SnapshotSecuritySubject, SourceEngine,
    ToolCallStatus, ToolOrigin, TraceSnapshot, VmLifecycleSecuritySubject,
    SECURITY_EVENT_SCHEMA_VERSION,
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

fn common_for(
    event_id: &str,
    event_type: &str,
    source_engine: SourceEngine,
) -> SecurityEventCommon {
    SecurityEventCommon {
        event_id: event_id.to_owned(),
        parent_event_id: None,
        stream_id: None,
        activity_id: None,
        sequence_no: Some(1),
        source_engine,
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
        event_type: SecurityEventType::parse(event_type).unwrap(),
        redaction_state: RedactionState::Raw,
    }
}

fn common(event_id: &str) -> SecurityEventCommon {
    common_for(event_id, "http.request", SourceEngine::Network)
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

fn model_tool_event() -> SecurityEvent {
    let evidence = ModelInteractionEvidence {
        interaction_id: "interaction-bench".into(),
        trace_id: "trace-bench".into(),
        attribution_scope: AiAttributionScope::Vm,
        source_engine: SourceEngine::Network,
        origin_kind: AiOriginKind::GuestNetwork,
        accounting_owner: Some("vm:bench-vm".into()),
        profile_id: Some("coding".into()),
        vm_id: Some("bench-vm".into()),
        session_id: Some("bench-session".into()),
        user_id: Some("bench-user".into()),
        provider: AiProvider::GoogleGemini,
        api_family: AiApiFamily::GoogleGeminiContent,
        model: "gemini-2.5-pro".into(),
        request: ModelRequestEvidence {
            request_id: "req-bench-model".into(),
            provider: AiProvider::GoogleGemini,
            api_family: AiApiFamily::GoogleGeminiContent,
            model: Some("gemini-2.5-pro".into()),
            stream: true,
            system_prompt_preview: Some("bench system prompt".into()),
            message_count: 3,
            tools_declared_count: 1,
            raw_shape_version: "gemini-content-v1".into(),
            unknown_fields_present: false,
        },
        response: Some(ModelResponseEvidence {
            response_id: "resp-bench-model".into(),
            provider_response_id: Some("provider-resp-1".into()),
            stop_reason: Some("tool_calls".into()),
            text_preview: Some("calling filesystem".into()),
            thinking_preview: None,
            content_blocks: Vec::new(),
            usage: AiUsageEvidence {
                input_tokens: Some(32),
                output_tokens: Some(16),
                estimated_cost_micros: Some(7),
                details: BTreeMap::new(),
            },
            raw_shape_version: "gemini-content-v1".into(),
        }),
        tool_calls: vec![ModelToolCallEvidence {
            tool_call_id: "tool-call-1".into(),
            index: 0,
            provider_call_id: Some("provider-call-1".into()),
            raw_name: "read_file".into(),
            normalized_name: "filesystem.read_file".into(),
            arguments_raw: Some(r#"{"path":"/workspace/secret.txt"}"#.into()),
            arguments_json: Some(r#"{"path":"/workspace/secret.txt"}"#.into()),
            arguments_status: ArgumentsStatus::ValidJson,
            origin: ToolOrigin::McpTool,
            linked_mcp_call_id: Some("mcp-call-1".into()),
            status: ToolCallStatus::Proposed,
            parse_confidence: Confidence::High,
        }],
        tool_results: vec![ModelToolResultEvidence {
            tool_call_id: "tool-call-1".into(),
            linked_mcp_call_id: Some("mcp-call-1".into()),
            content_kind: AiContentKind::Text,
            content_preview: Some("file contents".into()),
            content_json: None,
            is_error: false,
            result_status: ToolCallStatus::ReturnedToModel,
            returned_to_model: true,
            parse_confidence: Confidence::High,
        }],
        mcp_executions: Vec::new(),
        usage: AiUsageEvidence {
            input_tokens: Some(32),
            output_tokens: Some(16),
            estimated_cost_micros: Some(7),
            details: BTreeMap::new(),
        },
        parse_status: ParseStatus::Complete,
        evidence_status: EvidenceStatus::Complete,
    };

    SecurityEvent::model(
        common_for(
            "evt-bench-model-tool",
            "model.response",
            SourceEngine::Network,
        ),
        ModelSecuritySubject::from_interaction_evidence(evidence),
    )
}

fn event_family_cases() -> Vec<(&'static str, SecurityEvent, &'static str)> {
    vec![
        (
            "dns",
            SecurityEvent::dns(
                common_for("evt-bench-dns", "dns.request", SourceEngine::Network),
                DnsSecuritySubject {
                    qname: "google.example.test".into(),
                    domain_class: "external".into(),
                },
            ),
            "dns.request.qname == 'google.example.test'",
        ),
        (
            "http",
            http_event(),
            "http.request.host == 'googleapis.com'",
        ),
        (
            "mcp",
            SecurityEvent::mcp(
                common_for("evt-bench-mcp", "mcp.request", SourceEngine::Network),
                McpSecuritySubject {
                    method: Some("tools/call".into()),
                    server_id: "filesystem".into(),
                    tool_name: "read_file".into(),
                    evidence: None,
                },
            ),
            "mcp.request.tool_name == 'read_file'",
        ),
        (
            "model_tool",
            model_tool_event(),
            "model.request.tool_calls[0].name == 'filesystem.read_file' \
                && model.response.tool_results[0].returned_to_model",
        ),
        (
            "file",
            SecurityEvent::file(
                common_for("evt-bench-file", "file.write", SourceEngine::File),
                FileSecuritySubject {
                    operation: "write".into(),
                    path: Some("/workspace/secret.txt".into()),
                    path_class: "workspace".into(),
                    byte_count: Some(64),
                    content: None,
                },
            ),
            "file.activity.path_class == 'workspace'",
        ),
        (
            "process",
            SecurityEvent::process(
                common_for("evt-bench-process", "process.exec", SourceEngine::Process),
                ProcessSecuritySubject {
                    operation: "exec".into(),
                    command_class: Some("shell".into()),
                },
            ),
            "process.activity.command_class == 'shell'",
        ),
        (
            "credential",
            SecurityEvent {
                schema_version: SECURITY_EVENT_SCHEMA_VERSION,
                common: common_for(
                    "evt-bench-credential",
                    "credential.activity",
                    SourceEngine::Security,
                ),
                subject: SecurityEventSubject::Credential(CredentialSecuritySubject {
                    operation: "read".into(),
                    credential_id: "api-token".into(),
                }),
                context: EventContext::default(),
                trace: TraceSnapshot::default(),
                labels: Vec::new(),
                findings: Vec::new(),
                decision: None,
                mutations: Vec::new(),
            },
            "credential.activity.credential_id == 'api-token'",
        ),
        (
            "vm",
            SecurityEvent::vm_lifecycle(
                common_for("evt-bench-vm", "vm.start", SourceEngine::Vm),
                VmLifecycleSecuritySubject {
                    operation: "start".into(),
                },
            ),
            "vm.activity.operation == 'start'",
        ),
        (
            "profile",
            SecurityEvent::profile(
                common_for("evt-bench-profile", "profile.update", SourceEngine::Profile),
                ProfileSecuritySubject {
                    operation: "update".into(),
                    profile_id: "coding".into(),
                    profile_revision: "rev-a".into(),
                },
            ),
            "profile.activity.profile_id == 'coding'",
        ),
        (
            "conversation",
            SecurityEvent::conversation(
                common_for(
                    "evt-bench-conversation",
                    "conversation.message",
                    SourceEngine::Conversation,
                ),
                ConversationSecuritySubject {
                    operation: "append".into(),
                    conversation_id: Some("conv-1".into()),
                },
            ),
            "conversation.activity.conversation_id == 'conv-1'",
        ),
        (
            "snapshot",
            SecurityEvent::snapshot(
                common_for("evt-bench-snapshot", "snapshot.create", SourceEngine::File),
                SnapshotSecuritySubject {
                    operation: "create".into(),
                    snapshot_id: "snap-1".into(),
                },
            ),
            "snapshot.activity.snapshot_id == 'snap-1'",
        ),
    ]
}

fn rule(id: impl Into<String>, condition: impl Into<String>) -> CelEnforcementRule {
    CelEnforcementRule {
        id: id.into(),
        pack_id: Some("bench.enforcement".into()),
        condition: condition.into(),
        decision: SecurityDecisionAction::Block,
        reason: Some("benchmark match".into()),
        mutations: Vec::new(),
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

fn registry_detection_record(
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
        definition: RuntimeRuleDefinition::Detection {
            sigma_id: Some("sigma-bench".into()),
            title: "Benchmark registry detection".into(),
            severity: Severity::Medium,
            confidence: Confidence::High,
            tags: vec!["benchmark".into(), "http".into()],
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

fn mixed_family_evaluator(rule_count: usize) -> CelEnforcementEvaluator {
    let cases = event_family_cases();
    let rules = (0..rule_count)
        .map(|index| {
            let (family, _, condition) = cases[index % cases.len()];
            rule(format!("bench-mixed-{index:03}-{family}"), condition)
        })
        .collect();
    CelEnforcementEvaluator::compile(rules).unwrap()
}

fn mixed_family_detection_evaluator(rule_count: usize) -> CelDetectionEvaluator {
    let cases = event_family_cases();
    let rules = (0..rule_count)
        .map(|index| {
            let (family, _, condition) = cases[index % cases.len()];
            detection_rule(format!("bench-detect-mixed-{index:03}-{family}"), condition)
        })
        .collect();
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

fn backtest_inputs(count: usize) -> Vec<BacktestEventInput> {
    (0..count)
        .map(|index| {
            let mut event = http_event();
            event.common.event_id = format!("evt-runtime-backtest-{index}");
            event.common.sequence_no = Some(index as u64);
            BacktestEventInput {
                event_ref: Some(BacktestEventRef {
                    corpus: "criterion".into(),
                    session_id: event.common.session_id.clone(),
                    event_id: event.common.event_id.clone(),
                    sequence_no: event.common.sequence_no,
                    timestamp_unix_ms: event.common.timestamp_unix_ms,
                }),
                event,
                expected: Some("finding".into()),
            }
        })
        .collect()
}

fn registry_with_enforcement_rules(rule_count: usize) -> RuntimeRuleRegistry {
    let mut registry = RuntimeRuleRegistry::default();
    for index in 0..rule_count {
        registry
            .add_or_update(
                registry_enforcement_record(format!("bench-runtime-{index:03}"), "false"),
                |_| Ok::<_, RuleRegistryError>("compiled-plan".into()),
            )
            .unwrap();
    }
    registry
}

fn registry_with_detection_rules(rule_count: usize) -> RuntimeRuleRegistry {
    let mut registry = RuntimeRuleRegistry::default();
    for index in 0..rule_count {
        registry
            .add_or_update(
                registry_detection_record(format!("bench-detect-runtime-{index:03}"), "false"),
                |_| Ok::<_, RuleRegistryError>("compiled-plan".into()),
            )
            .unwrap();
    }
    registry
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

fn bench_event_family_evaluate(c: &mut Criterion) {
    let cases = event_family_cases();
    let mut group = c.benchmark_group("security_engine_cel_evaluate_event_families");

    for (family, event, condition) in &cases {
        let mut enforcement = evaluator(condition);
        group.bench_function(format!("enforcement_{family}"), |b| {
            b.iter(|| {
                let decision = enforcement.evaluate(black_box(event)).unwrap();
                black_box(decision.is_some())
            });
        });

        let mut detection = detection_evaluator(condition);
        group.bench_function(format!("detection_{family}"), |b| {
            b.iter(|| {
                let findings = detection.evaluate(black_box(event)).unwrap();
                black_box(findings.len())
            });
        });
    }

    let model_event = model_tool_event();
    let mut mixed_enforcement = mixed_family_evaluator(100);
    group.bench_function("enforcement_mixed_family_100_rules_model_tool_event", |b| {
        b.iter(|| {
            let decision = mixed_enforcement.evaluate(black_box(&model_event)).unwrap();
            black_box(decision.is_some())
        });
    });

    let mut mixed_detection = mixed_family_detection_evaluator(100);
    group.bench_function("detection_mixed_family_100_rules_model_tool_event", |b| {
        b.iter(|| {
            let findings = mixed_detection.evaluate(black_box(&model_event)).unwrap();
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

fn bench_runtime_backtest_and_hunt(c: &mut Criterion) {
    let events_10 = backtest_inputs(10);
    let events_100 = backtest_inputs(100);
    let mut detection_rules = Vec::with_capacity(100);
    for index in 0..99 {
        detection_rules.push(detection_rule(
            format!("bench-hunt-no-match-{index}"),
            "false",
        ));
    }
    detection_rules.push(detection_rule("bench-hunt-match", CANONICAL_HTTP_POLICY));

    let mut group = c.benchmark_group("security_engine_runtime_backtest_hunt");

    group.bench_function("enforcement_backtest_10_events", |b| {
        b.iter(|| {
            let result = run_enforcement_backtest(
                black_box(rule("bench-enforce-backtest", CANONICAL_HTTP_POLICY)),
                black_box(&events_10),
                Some(100),
            )
            .unwrap();
            black_box(result.total_matches)
        });
    });

    group.bench_function("detection_backtest_10_events", |b| {
        b.iter(|| {
            let result = run_detection_backtest(
                black_box(detection_rule(
                    "bench-detect-backtest",
                    CANONICAL_HTTP_POLICY,
                )),
                black_box(&events_10),
                Some(100),
            )
            .unwrap();
            black_box(result.total_matches)
        });
    });

    group.bench_function("detection_hunt_100_rules_100_events", |b| {
        b.iter(|| {
            let result = run_detection_hunt(
                black_box(detection_rules.clone()),
                black_box(&events_100),
                Some(100),
            )
            .unwrap();
            black_box(result.total_matches)
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
        let registry = registry_with_enforcement_rules(100);
        b.iter(|| black_box(registry.enabled_enforcement_rules().len()));
    });

    group.bench_function("project_and_compile_enforcement_100_rules", |b| {
        let registry = registry_with_enforcement_rules(100);
        b.iter(|| {
            let rules = registry.enabled_enforcement_rules();
            let evaluator = CelEnforcementEvaluator::compile(black_box(rules)).unwrap();
            black_box(evaluator)
        });
    });

    group.bench_function("project_and_compile_detection_100_rules", |b| {
        let registry = registry_with_detection_rules(100);
        b.iter(|| {
            let rules = registry.enabled_detection_rules();
            let evaluator = CelDetectionEvaluator::compile(black_box(rules)).unwrap();
            black_box(evaluator)
        });
    });

    group.bench_function("rebuild_engine_from_100_enforcement_100_detection", |b| {
        let enforcement_registry = registry_with_enforcement_rules(100);
        let detection_registry = registry_with_detection_rules(100);
        b.iter(|| {
            let mut engine = SecurityEngine::default();
            let enforcement = CelEnforcementEvaluator::compile(black_box(
                enforcement_registry.enabled_enforcement_rules(),
            ))
            .unwrap();
            let detection = CelDetectionEvaluator::compile(black_box(
                detection_registry.enabled_detection_rules(),
            ))
            .unwrap();
            engine.set_enforcement(Box::new(enforcement));
            engine.set_detection(Box::new(detection));
            black_box(engine)
        });
    });

    group.bench_function("update_existing_then_rebuild_100_rule_plan", |b| {
        let baseline = registry_with_enforcement_rules(100);
        let mut generation = 0_u64;
        b.iter(|| {
            generation += 1;
            let mut registry = baseline.clone();
            registry
                .add_or_update(
                    registry_enforcement_record(
                        "bench-runtime-050",
                        format!("{} && true", CANONICAL_HTTP_POLICY),
                    ),
                    |_| Ok::<_, RuleRegistryError>(format!("compiled-plan-{generation}")),
                )
                .unwrap();
            let evaluator =
                CelEnforcementEvaluator::compile(black_box(registry.enabled_enforcement_rules()))
                    .unwrap();
            black_box(evaluator)
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

    let cases = event_family_cases();
    for (family, event, _) in &cases {
        group.bench_function(
            format!("project_security_event_to_policy_context_{family}"),
            |b| {
                b.iter(|| black_box(policy_context_from_event(black_box(event))));
            },
        );
    }
    group.bench_function("project_all_event_families_to_policy_context", |b| {
        b.iter(|| {
            for (_, event, _) in &cases {
                black_box(policy_context_from_event(black_box(event)));
            }
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
    bench_event_family_evaluate,
    bench_backtest_dedupe,
    bench_runtime_backtest_and_hunt,
    bench_runtime_registry,
    bench_materialization,
    bench_native_lookup
);
criterion_main!(benches);
