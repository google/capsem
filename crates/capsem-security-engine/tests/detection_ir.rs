use std::path::{Path, PathBuf};

use capsem_security_engine::detection_ir::{
    compile_detection_ir_to_cel_detection_rules, evaluate_detection_ir,
    evaluate_detection_ir_security_event, parse_detection_ir_v1_json,
    validate_detection_ir_v1_json, DetectionIRMatcherV1, DetectionOperator, EventFamily,
    SecurityEventV1, SecurityPackSchemaError,
};
use capsem_security_engine::{
    AiAttributionScope, AiOriginKind, CelDetectionEvaluator, ConversationSecuritySubject,
    CredentialSecuritySubject, DetectionEvaluator, DnsSecuritySubject, Enforceability,
    FileSecuritySubject, HttpBodySecuritySubject, HttpSecuritySubject, McpSecuritySubject,
    ModelSecuritySubject, ProcessSecuritySubject, ProfileSecuritySubject, RedactionState,
    SecurityEvent, SecurityEventCommon, SecurityEventSubject, SecurityEventType,
    SnapshotSecuritySubject, SourceEngine, VmLifecycleSecuritySubject,
};
use serde_json::Value;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect(
            "capsem-security-engine crate should live under <repo>/crates/capsem-security-engine",
        )
        .to_path_buf()
}

fn fixture(name: &str) -> String {
    let path = repo_root().join("schemas/fixtures").join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
}

fn common(event_id: &str, event_type: &str, source_engine: SourceEngine) -> SecurityEventCommon {
    SecurityEventCommon {
        event_id: event_id.into(),
        parent_event_id: None,
        stream_id: None,
        activity_id: Some(format!("{event_id}-activity")),
        sequence_no: Some(1),
        source_engine,
        attribution_scope: AiAttributionScope::Vm,
        origin_kind: AiOriginKind::GuestNetwork,
        accounting_owner: Some("vm:vm-1".into()),
        enforceability: Enforceability::InlineBlockable,
        trace_id: Some(format!("{event_id}-trace")),
        span_id: None,
        timestamp_unix_ms: 1_789,
        vm_id: Some("vm-1".into()),
        session_id: Some("session-1".into()),
        profile_id: Some("coding".into()),
        profile_revision: Some("rev-a".into()),
        profile_pack_ids: vec!["corp-default-detections".into()],
        enforcement_packs: Vec::new(),
        detection_packs: Vec::new(),
        user_id: Some("user-1".into()),
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

#[test]
fn detection_ir_schema_accepts_valid_golden_fixture() {
    let value = validate_detection_ir_v1_json(&fixture("detection-ir-v1-valid.json")).unwrap();

    assert_eq!(value["schema"], "capsem.detection.ir.v1");
    assert_eq!(value["pack_id"], "corp-default-detections");
    assert_eq!(value["rules"][0]["event_family"], "http");
}

#[test]
fn detection_ir_schema_rejects_invalid_golden_fixture() {
    let error = validate_detection_ir_v1_json(&fixture("detection-ir-v1-invalid-extra-field.json"))
        .unwrap_err();

    assert!(matches!(error, SecurityPackSchemaError::Validation(_)));
    assert!(error.to_string().contains("Additional properties"));
}

#[test]
fn detection_ir_typed_parser_rejects_unknown_fields() {
    let mut value: Value = serde_json::from_str(&fixture("detection-ir-v1-valid.json")).unwrap();
    value["rules"][0]["matchers"][0]["extra"] = serde_json::json!("nope");

    let error = parse_detection_ir_v1_json(&serde_json::to_string(&value).unwrap()).unwrap_err();

    assert!(matches!(error, SecurityPackSchemaError::ParseJson(_)));
    assert!(error.to_string().contains("unknown field"));
}

#[test]
fn detection_ir_evaluator_matches_normalized_event() {
    let ir = parse_detection_ir_v1_json(&fixture("detection-ir-v1-valid.json")).unwrap();
    let event: SecurityEventV1 = serde_json::from_value(serde_json::json!({
        "event_id": "evt-1",
        "event_family": "http",
        "event_type": "http.request",
        "subject": {
            "request": {
                "host": "169.254.169.254",
                "url": "http://169.254.169.254/latest"
            }
        }
    }))
    .unwrap();

    let findings = evaluate_detection_ir(&ir, &event);

    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].event_id, "evt-1");
    assert_eq!(findings[0].rule_id, "metadata-access");
    assert_eq!(
        findings[0].matched_fields["http.request.host"],
        serde_json::json!("169.254.169.254")
    );
}

#[test]
fn detection_ir_evaluator_matches_security_engine_http_event() {
    let ir = parse_detection_ir_v1_json(&fixture("detection-ir-v1-valid.json")).unwrap();
    let event = SecurityEvent::http(
        SecurityEventCommon {
            event_id: "evt-s08b-http".into(),
            parent_event_id: None,
            stream_id: Some("http-stream-1".into()),
            activity_id: Some("http-request-1".into()),
            sequence_no: Some(1),
            source_engine: capsem_security_engine::SourceEngine::Network,
            attribution_scope: capsem_security_engine::AiAttributionScope::Vm,
            origin_kind: capsem_security_engine::AiOriginKind::GuestNetwork,
            accounting_owner: Some("vm:vm-1".into()),
            enforceability: capsem_security_engine::Enforceability::InlineBlockable,
            trace_id: Some("trace-s08b".into()),
            span_id: None,
            timestamp_unix_ms: 1_789,
            vm_id: Some("vm-1".into()),
            session_id: Some("session-1".into()),
            profile_id: Some("coding".into()),
            profile_revision: Some("rev-a".into()),
            profile_pack_ids: vec!["corp-default-detections".into()],
            enforcement_packs: Vec::new(),
            detection_packs: Vec::new(),
            user_id: Some("user-1".into()),
            process_id: None,
            parent_process_id: None,
            exec_id: None,
            turn_id: None,
            message_id: None,
            tool_call_id: None,
            mcp_call_id: None,
            event_type: SecurityEventType::HttpRequest,
            redaction_state: RedactionState::Raw,
        },
        HttpSecuritySubject {
            method: "GET".into(),
            host: "169.254.169.254".into(),
            path_class: "metadata".into(),
            request_bytes: 128,
            response_bytes: None,
            ..Default::default()
        },
    );

    let findings = evaluate_detection_ir_security_event(&ir, &event);

    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].event_id, "evt-s08b-http");
    assert_eq!(
        findings[0].matched_fields["http.request.host"],
        serde_json::json!("169.254.169.254")
    );
}

#[test]
fn detection_ir_lowers_to_real_cel_detection_rules() {
    let ir = parse_detection_ir_v1_json(&fixture("detection-ir-v1-valid.json")).unwrap();
    let rules = compile_detection_ir_to_cel_detection_rules(&ir).unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].id, "metadata-access");
    assert_eq!(rules[0].pack_id, "corp-default-detections");
    assert_eq!(
        rules[0].sigma_id.as_deref(),
        Some("11111111-1111-4111-8111-111111111111")
    );
    assert!(rules[0]
        .condition
        .contains("common.event_type.startsWith(\"http.\")"));
    assert!(rules[0]
        .condition
        .contains("http.request.host == \"169.254.169.254\""));

    let event = SecurityEvent::http(
        SecurityEventCommon {
            event_id: "evt-cel-ir-http".into(),
            parent_event_id: None,
            stream_id: Some("http-stream-1".into()),
            activity_id: Some("http-request-1".into()),
            sequence_no: Some(1),
            source_engine: capsem_security_engine::SourceEngine::Network,
            attribution_scope: capsem_security_engine::AiAttributionScope::Vm,
            origin_kind: capsem_security_engine::AiOriginKind::GuestNetwork,
            accounting_owner: Some("vm:vm-1".into()),
            enforceability: capsem_security_engine::Enforceability::InlineBlockable,
            trace_id: Some("trace-s08b".into()),
            span_id: None,
            timestamp_unix_ms: 1_789,
            vm_id: Some("vm-1".into()),
            session_id: Some("session-1".into()),
            profile_id: Some("coding".into()),
            profile_revision: Some("rev-a".into()),
            profile_pack_ids: vec!["corp-default-detections".into()],
            enforcement_packs: Vec::new(),
            detection_packs: Vec::new(),
            user_id: Some("user-1".into()),
            process_id: None,
            parent_process_id: None,
            exec_id: None,
            turn_id: None,
            message_id: None,
            tool_call_id: None,
            mcp_call_id: None,
            event_type: SecurityEventType::HttpRequest,
            redaction_state: RedactionState::Raw,
        },
        HttpSecuritySubject {
            method: "GET".into(),
            host: "169.254.169.254".into(),
            path_class: "metadata".into(),
            request_bytes: 128,
            response_bytes: None,
            ..Default::default()
        },
    );

    let mut evaluator = CelDetectionEvaluator::compile(rules).unwrap();
    let findings = evaluator.evaluate(&event).unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].event_id, "evt-cel-ir-http");
    assert_eq!(findings[0].rule_id, "metadata-access");
    assert_eq!(findings[0].pack_id, "corp-default-detections");
}

#[test]
fn detection_ir_lowering_rejects_legacy_subject_paths() {
    let mut ir = parse_detection_ir_v1_json(&fixture("detection-ir-v1-valid.json")).unwrap();
    ir.rules[0].matchers[0].field_path = "subject.request.host".into();

    let error = compile_detection_ir_to_cel_detection_rules(&ir).unwrap_err();

    assert!(matches!(
        error,
        SecurityPackSchemaError::UnsupportedDetectionIr(_)
    ));
    assert!(error.to_string().contains("subject.request.host"));
}

#[test]
fn s08c_detection_expected_artifact_matches_rust_detection_ir() {
    let ir = parse_detection_ir_v1_json(include_str!(
        "../../../data/detection/ir/google-secret-egress.json"
    ))
    .unwrap();
    let fixtures: Vec<Value> =
        include_str!("../../../data/policy-context/canonical-policy-contexts.jsonl")
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
    let mut findings = Vec::new();

    for fixture in &fixtures {
        let event: SecurityEventV1 = serde_json::from_value(serde_json::json!({
            "event_id": fixture["event_ref"]["event_id"],
            "event_family": "http",
            "event_type": fixture["context"]["common"]["event_type"],
            "subject": {
                "request": fixture["context"]["http"]["request"]
            }
        }))
        .unwrap();
        findings.extend(
            evaluate_detection_ir(&ir, &event)
                .into_iter()
                .map(|finding| serde_json::to_value(finding).unwrap()),
        );
    }

    let actual = serde_json::json!({
        "schema": "capsem.detection-check.v1",
        "ok": true,
        "pack_id": ir.pack_id,
        "pack_version": ir.pack_version,
        "event_count": fixtures.len(),
        "rule_count": ir.rules.len(),
        "match_count": findings.len(),
        "findings": findings,
        "diagnostics": [],
    });
    let expected: Value = serde_json::from_str(include_str!(
        "../../../data/detection/backtest-expected/google-secret-egress.json"
    ))
    .unwrap();

    assert_eq!(actual, expected);
}

#[test]
fn detection_ir_lowers_http_url_path_and_body_to_policy_context_roots() {
    let mut value: Value = serde_json::from_str(&fixture("detection-ir-v1-valid.json")).unwrap();
    value["rules"][0]["matchers"] = serde_json::json!([
        {
            "field_path": "http.request.url",
            "operator": "equals_any",
            "values": ["https://google.example.test/admin/settings"],
            "sigma_field": "url"
        },
        {
            "field_path": "http.request.path",
            "operator": "equals_any",
            "values": ["/admin/settings"],
            "sigma_field": "path"
        },
        {
            "field_path": "http.request.body.text",
            "operator": "equals_any",
            "values": ["secret"],
            "sigma_field": "body"
        }
    ]);
    let ir = parse_detection_ir_v1_json(&serde_json::to_string(&value).unwrap()).unwrap();
    let rules = compile_detection_ir_to_cel_detection_rules(&ir).unwrap();
    assert!(rules[0]
        .condition
        .contains("http.request.url == \"https://google.example.test/admin/settings\""));
    assert!(rules[0]
        .condition
        .contains("http.request.path == \"/admin/settings\""));
    assert!(rules[0]
        .condition
        .contains("http.request.body.text == \"secret\""));

    let event = SecurityEvent::http(
        SecurityEventCommon {
            event_id: "evt-http-full-surface".into(),
            parent_event_id: None,
            stream_id: Some("http-stream-1".into()),
            activity_id: Some("http-request-1".into()),
            sequence_no: Some(1),
            source_engine: capsem_security_engine::SourceEngine::Network,
            attribution_scope: capsem_security_engine::AiAttributionScope::Vm,
            origin_kind: capsem_security_engine::AiOriginKind::GuestNetwork,
            accounting_owner: Some("vm:vm-1".into()),
            enforceability: capsem_security_engine::Enforceability::InlineBlockable,
            trace_id: Some("trace-s08b".into()),
            span_id: None,
            timestamp_unix_ms: 1_789,
            vm_id: Some("vm-1".into()),
            session_id: Some("session-1".into()),
            profile_id: Some("coding".into()),
            profile_revision: Some("rev-a".into()),
            profile_pack_ids: vec!["corp-default-detections".into()],
            enforcement_packs: Vec::new(),
            detection_packs: Vec::new(),
            user_id: Some("user-1".into()),
            process_id: None,
            parent_process_id: None,
            exec_id: None,
            turn_id: None,
            message_id: None,
            tool_call_id: None,
            mcp_call_id: None,
            event_type: SecurityEventType::HttpRequest,
            redaction_state: RedactionState::Raw,
        },
        HttpSecuritySubject {
            method: "POST".into(),
            host: "google.example.test".into(),
            path: Some("/admin/settings".into()),
            url: Some("https://google.example.test/admin/settings".into()),
            path_class: "admin".into(),
            request_bytes: 128,
            request_body: Some(HttpBodySecuritySubject::text("secret")),
            response_bytes: None,
            ..Default::default()
        },
    );
    let mut evaluator = CelDetectionEvaluator::compile(rules).unwrap();
    let findings = evaluator.evaluate(&event).unwrap();
    assert_eq!(findings.len(), 1);
}

#[test]
fn detection_ir_lowers_file_path_to_policy_context_roots() {
    let mut ir = parse_detection_ir_v1_json(&fixture("detection-ir-v1-valid.json")).unwrap();
    let rule = &mut ir.rules[0];
    rule.id = "workspace-file-write".into();
    rule.event_family = EventFamily::File;
    rule.matchers = vec![
        DetectionIRMatcherV1 {
            field_path: "file.activity.operation".into(),
            operator: DetectionOperator::EqualsAny,
            values: vec![serde_json::json!("write")],
            sigma_field: "operation".into(),
        },
        DetectionIRMatcherV1 {
            field_path: "file.activity.path".into(),
            operator: DetectionOperator::EqualsAny,
            values: vec![serde_json::json!("/workspace/secret.txt")],
            sigma_field: "path".into(),
        },
        DetectionIRMatcherV1 {
            field_path: "file.activity.path_class".into(),
            operator: DetectionOperator::EqualsAny,
            values: vec![serde_json::json!("workspace")],
            sigma_field: "path_class".into(),
        },
    ];

    let rules = compile_detection_ir_to_cel_detection_rules(&ir).unwrap();
    assert!(rules[0]
        .condition
        .contains("file.activity.path == \"/workspace/secret.txt\""));

    let event = SecurityEvent::file(
        SecurityEventCommon {
            event_id: "evt-file-path".into(),
            parent_event_id: None,
            stream_id: None,
            activity_id: Some("file-write-1".into()),
            sequence_no: Some(1),
            source_engine: capsem_security_engine::SourceEngine::File,
            attribution_scope: capsem_security_engine::AiAttributionScope::Vm,
            origin_kind: capsem_security_engine::AiOriginKind::GuestNetwork,
            accounting_owner: Some("vm:vm-1".into()),
            enforceability: capsem_security_engine::Enforceability::InlineBlockable,
            trace_id: Some("trace-file".into()),
            span_id: None,
            timestamp_unix_ms: 1_790,
            vm_id: Some("vm-1".into()),
            session_id: Some("session-1".into()),
            profile_id: Some("coding".into()),
            profile_revision: Some("rev-a".into()),
            profile_pack_ids: vec!["corp-default-detections".into()],
            enforcement_packs: Vec::new(),
            detection_packs: Vec::new(),
            user_id: Some("user-1".into()),
            process_id: None,
            parent_process_id: None,
            exec_id: None,
            turn_id: None,
            message_id: None,
            tool_call_id: None,
            mcp_call_id: None,
            event_type: SecurityEventType::FileWrite,
            redaction_state: RedactionState::Raw,
        },
        FileSecuritySubject {
            operation: "write".into(),
            path: Some("/workspace/secret.txt".into()),
            path_class: "workspace".into(),
            byte_count: Some(64),
            content: None,
        },
    );
    let mut evaluator = CelDetectionEvaluator::compile(rules).unwrap();
    let findings = evaluator.evaluate(&event).unwrap();
    assert_eq!(findings.len(), 1);
}

#[test]
fn detection_ir_lowers_every_security_event_family_to_cel_roots() {
    let cases = vec![
        (
            EventFamily::Dns,
            "dns.request.qname",
            serde_json::json!("google.example.test"),
            SecurityEvent::dns(
                common("evt-ir-dns", "dns.request", SourceEngine::Network),
                DnsSecuritySubject {
                    qname: "google.example.test".into(),
                    domain_class: "external".into(),
                },
            ),
        ),
        (
            EventFamily::Http,
            "http.request.host",
            serde_json::json!("api.example.test"),
            SecurityEvent::http(
                common("evt-ir-http", "http.request", SourceEngine::Network),
                HttpSecuritySubject {
                    method: "GET".into(),
                    host: "api.example.test".into(),
                    path_class: "api".into(),
                    request_bytes: 64,
                    ..Default::default()
                },
            ),
        ),
        (
            EventFamily::Mcp,
            "mcp.request.tool_name",
            serde_json::json!("read_file"),
            SecurityEvent::mcp(
                common("evt-ir-mcp", "mcp.request", SourceEngine::Network),
                McpSecuritySubject {
                    method: Some("tools/call".into()),
                    server_id: "filesystem".into(),
                    tool_name: "read_file".into(),
                    evidence: None,
                },
            ),
        ),
        (
            EventFamily::Model,
            "model.request.provider",
            serde_json::json!("google_gemini"),
            SecurityEvent::model(
                common("evt-ir-model", "model.request", SourceEngine::Network),
                ModelSecuritySubject {
                    provider: "google_gemini".into(),
                    model: "gemini-2.5-pro".into(),
                    estimated_input_tokens: None,
                    estimated_output_tokens: None,
                    estimated_cost_micros: None,
                    evidence: None,
                },
            ),
        ),
        (
            EventFamily::File,
            "file.activity.path_class",
            serde_json::json!("workspace"),
            SecurityEvent::file(
                common("evt-ir-file", "file.write", SourceEngine::File),
                FileSecuritySubject {
                    operation: "write".into(),
                    path: Some("/workspace/secret.txt".into()),
                    path_class: "workspace".into(),
                    byte_count: Some(64),
                    content: None,
                },
            ),
        ),
        (
            EventFamily::Process,
            "process.activity.command_class",
            serde_json::json!("shell"),
            SecurityEvent::process(
                common("evt-ir-process", "process.exec", SourceEngine::Process),
                ProcessSecuritySubject {
                    operation: "exec".into(),
                    command_class: Some("shell".into()),
                },
            ),
        ),
        (
            EventFamily::Credential,
            "credential.activity.credential_id",
            serde_json::json!("api-token"),
            SecurityEvent {
                schema_version: capsem_security_engine::SECURITY_EVENT_SCHEMA_VERSION,
                common: common(
                    "evt-ir-credential",
                    "credential.activity",
                    SourceEngine::Security,
                ),
                subject: SecurityEventSubject::Credential(CredentialSecuritySubject {
                    operation: "read".into(),
                    credential_id: "api-token".into(),
                }),
                context: Default::default(),
                trace: Default::default(),
                labels: Vec::new(),
                findings: Vec::new(),
                decision: None,
                mutations: Vec::new(),
            },
        ),
        (
            EventFamily::Vm,
            "vm.activity.operation",
            serde_json::json!("start"),
            SecurityEvent::vm_lifecycle(
                common("evt-ir-vm", "vm.start", SourceEngine::Vm),
                VmLifecycleSecuritySubject {
                    operation: "start".into(),
                },
            ),
        ),
        (
            EventFamily::Profile,
            "profile.activity.profile_id",
            serde_json::json!("coding"),
            SecurityEvent::profile(
                common("evt-ir-profile", "profile.update", SourceEngine::Profile),
                ProfileSecuritySubject {
                    operation: "update".into(),
                    profile_id: "coding".into(),
                    profile_revision: "rev-a".into(),
                },
            ),
        ),
        (
            EventFamily::Conversation,
            "conversation.activity.conversation_id",
            serde_json::json!("conv-1"),
            SecurityEvent::conversation(
                common(
                    "evt-ir-conversation",
                    "conversation.message",
                    SourceEngine::Conversation,
                ),
                ConversationSecuritySubject {
                    operation: "append".into(),
                    conversation_id: Some("conv-1".into()),
                },
            ),
        ),
        (
            EventFamily::Snapshot,
            "snapshot.activity.snapshot_id",
            serde_json::json!("snap-1"),
            SecurityEvent::snapshot(
                common("evt-ir-snapshot", "snapshot.create", SourceEngine::File),
                SnapshotSecuritySubject {
                    operation: "create".into(),
                    snapshot_id: "snap-1".into(),
                },
            ),
        ),
    ];

    for (family, field_path, value, event) in cases {
        let mut ir = parse_detection_ir_v1_json(&fixture("detection-ir-v1-valid.json")).unwrap();
        let rule = &mut ir.rules[0];
        rule.id = format!("detect-{family:?}").to_lowercase();
        rule.event_family = family;
        rule.matchers = vec![DetectionIRMatcherV1 {
            field_path: field_path.into(),
            operator: DetectionOperator::EqualsAny,
            values: vec![value],
            sigma_field: field_path.into(),
        }];

        let rules = compile_detection_ir_to_cel_detection_rules(&ir).unwrap();
        assert!(
            rules[0].condition.contains(field_path),
            "lowered CEL should reference {field_path}"
        );
        let mut evaluator = CelDetectionEvaluator::compile(rules).unwrap();
        let findings = evaluator.evaluate(&event).unwrap();
        assert_eq!(findings.len(), 1, "expected {family:?} IR rule to match");
    }
}

#[test]
fn detection_ir_lowers_indexed_model_tool_paths_to_cel_roots() {
    let mut ir = parse_detection_ir_v1_json(&fixture("detection-ir-v1-valid.json")).unwrap();
    let rule = &mut ir.rules[0];
    rule.id = "model-tool-call".into();
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

    let rules = compile_detection_ir_to_cel_detection_rules(&ir).unwrap();
    assert!(rules[0]
        .condition
        .contains("model.request.tool_calls[0].name"));
    assert!(rules[0]
        .condition
        .contains("model.response.tool_results[0].returned_to_model"));
}

#[test]
fn detection_ir_lowering_rejects_unsupported_runtime_field_paths() {
    let mut ir = parse_detection_ir_v1_json(&fixture("detection-ir-v1-valid.json")).unwrap();
    ir.rules[0].matchers[0].field_path = "http.request.raw.unsupported".into();

    let error = compile_detection_ir_to_cel_detection_rules(&ir).unwrap_err();

    assert!(matches!(
        error,
        SecurityPackSchemaError::UnsupportedDetectionIr(_)
    ));
    assert!(error
        .to_string()
        .contains("unsupported Detection IR field path"));
}

#[test]
fn detection_ir_evaluator_ignores_nonmatching_event() {
    let ir = parse_detection_ir_v1_json(&fixture("detection-ir-v1-valid.json")).unwrap();
    let event: SecurityEventV1 = serde_json::from_value(serde_json::json!({
        "event_id": "evt-2",
        "event_family": "http",
        "event_type": "http.request",
        "subject": {
            "request": {
                "host": "example.com",
                "url": "https://example.com"
            }
        }
    }))
    .unwrap();

    assert!(evaluate_detection_ir(&ir, &event).is_empty());
}
