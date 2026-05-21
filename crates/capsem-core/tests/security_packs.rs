use std::path::{Path, PathBuf};

use capsem_core::security_packs::{
    evaluate_detection_ir, evaluate_detection_ir_security_event, parse_detection_ir_v1_json,
    validate_detection_ir_v1_json, SecurityEventV1, SecurityPackSchemaError,
};
use capsem_security_engine::{
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
            trace_id: Some("trace-s08b".into()),
            span_id: None,
            timestamp_unix_ms: 1_789,
            vm_id: Some("vm-1".into()),
            session_id: Some("session-1".into()),
            profile_id: Some("coding".into()),
            profile_revision: Some("rev-a".into()),
            profile_pack_ids: vec!["corp-default-detections".into()],
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
