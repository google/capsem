use std::path::{Path, PathBuf};

use capsem_core::security_packs::{
    compile_detection_ir_to_cel_detection_rules, evaluate_detection_ir,
    evaluate_detection_ir_security_event, parse_detection_ir_v1_json,
    validate_detection_ir_v1_json, DetectionIRMatcherV1, DetectionOperator, EventFamily,
    SecurityEventV1, SecurityPackSchemaError,
};
use capsem_security_engine::{
    CelDetectionEvaluator, DetectionEvaluator, FileSecuritySubject, HttpBodySecuritySubject,
    HttpSecuritySubject, RedactionState, SecurityEvent, SecurityEventCommon,
};
use serde_json::Value;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("capsem-core crate should live under <repo>/crates/capsem-core")
        .to_path_buf()
}

fn fixture(name: &str) -> String {
    let path = repo_root().join("schemas/fixtures").join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
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
        findings[0].matched_fields["subject.request.host"],
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
            event_type: "http.request".into(),
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
        findings[0].matched_fields["subject.request.host"],
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
            event_type: "http.request".into(),
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
fn detection_ir_lowers_http_url_path_and_body_to_policy_context_roots() {
    let mut value: Value = serde_json::from_str(&fixture("detection-ir-v1-valid.json")).unwrap();
    value["rules"][0]["matchers"] = serde_json::json!([
        {
            "field_path": "subject.request.url",
            "operator": "equals_any",
            "values": ["https://google.example.test/admin/settings"],
            "sigma_field": "url"
        },
        {
            "field_path": "subject.request.path",
            "operator": "equals_any",
            "values": ["/admin/settings"],
            "sigma_field": "path"
        },
        {
            "field_path": "subject.request.body.text",
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
            event_type: "http.request".into(),
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
            field_path: "subject.activity.operation".into(),
            operator: DetectionOperator::EqualsAny,
            values: vec![serde_json::json!("write")],
            sigma_field: "operation".into(),
        },
        DetectionIRMatcherV1 {
            field_path: "subject.activity.path".into(),
            operator: DetectionOperator::EqualsAny,
            values: vec![serde_json::json!("/workspace/secret.txt")],
            sigma_field: "path".into(),
        },
        DetectionIRMatcherV1 {
            field_path: "subject.activity.path_class".into(),
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
            event_type: "file.write".into(),
            redaction_state: RedactionState::Raw,
        },
        FileSecuritySubject {
            operation: "write".into(),
            path: Some("/workspace/secret.txt".into()),
            path_class: "workspace".into(),
            byte_count: Some(64),
        },
    );
    let mut evaluator = CelDetectionEvaluator::compile(rules).unwrap();
    let findings = evaluator.evaluate(&event).unwrap();
    assert_eq!(findings.len(), 1);
}

#[test]
fn detection_ir_lowering_rejects_unsupported_runtime_field_paths() {
    let mut ir = parse_detection_ir_v1_json(&fixture("detection-ir-v1-valid.json")).unwrap();
    ir.rules[0].matchers[0].field_path = "subject.request.raw.unsupported".into();

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
