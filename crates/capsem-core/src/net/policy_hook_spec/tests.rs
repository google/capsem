//! Tests for `policy_hook_spec`.

use super::*;
use serde_json::{json, Value};

#[test]
fn hook_decision_rejects_unknown_values() {
    let err = serde_json::from_str::<HookDecision>(r#""warn""#).unwrap_err();
    assert!(err.to_string().contains("unknown variant"));
}

#[test]
fn openapi_contains_every_callback_and_decision() {
    let doc = policy_hook_openapi_document();
    let callbacks = doc["components"]["schemas"]["HookCallback"]["enum"]
        .as_array()
        .expect("callback enum");
    for callback in [
        "mcp.request",
        "mcp.response",
        "http.request",
        "http.response",
        "dns.query",
        "dns.response",
        "model.request",
        "model.response",
        "model.tool_call",
        "model.tool_response",
    ] {
        assert!(
            callbacks.iter().any(|value| value == callback),
            "missing callback {callback}"
        );
    }

    let decisions = doc["components"]["schemas"]["HookDecision"]["enum"]
        .as_array()
        .expect("decision enum");
    for decision in ["allow", "ask", "block", "rewrite"] {
        assert!(
            decisions.iter().any(|value| value == decision),
            "missing decision {decision}"
        );
    }

    let response = &doc["components"]["schemas"]["HookDecisionResponse"]["properties"];
    assert!(response.get("rewrite_target").is_some());
    assert!(response.get("rewrite_value").is_some());
}

#[test]
fn checked_in_artifact_matches_rust_export() {
    let expected = include_str!("../../../../../config/policy-hook-openapi.json");
    let expected: Value = serde_json::from_str(expected).unwrap();
    assert_eq!(expected, policy_hook_openapi_document());
}

#[test]
fn sample_request_response_round_trip() {
    let request = HookDecisionRequest {
        spec_version: POLICY_HOOK_SPEC_VERSION.to_string(),
        decision_id: "decision-1".to_string(),
        trace_id: Some("trace-abc".to_string()),
        session_id: Some("vm-1".to_string()),
        on: HookCallback::HttpRequest,
        subject: json!({"request": {"host": "example.com", "path": "/"}}),
        preview: None,
        hashes: None,
        audit_context: Some(HookAuditContext {
            process_name: Some("curl".to_string()),
            pid: Some(42),
            provider: None,
            server_name: None,
            domain: Some("example.com".to_string()),
            config_source: Some("user".to_string()),
        }),
    };
    let encoded = serde_json::to_string(&request).unwrap();
    let decoded: HookDecisionRequest = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded.on, HookCallback::HttpRequest);

    let response = HookDecisionResponse {
        decision: HookDecision::Rewrite,
        decision_id: Some("decision-1".to_string()),
        rule_id: Some("hook.fixture".to_string()),
        priority: Some(10),
        reason: Some("redact token".to_string()),
        ttl_ms: Some(100),
        rewrite_target: Some("request.headers.authorization".to_string()),
        rewrite_value: Some("[redacted]".to_string()),
        redactions: vec!["rewrite_value".to_string()],
        audit_tags: vec!["fixture".to_string()],
    };
    let encoded = serde_json::to_string(&response).unwrap();
    let decoded: HookDecisionResponse = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded.decision, HookDecision::Rewrite);
    assert_eq!(decoded.rewrite_value.as_deref(), Some("[redacted]"));
}

#[test]
fn request_semantics_require_object_payloads() {
    let mut request = HookDecisionRequest {
        spec_version: POLICY_HOOK_SPEC_VERSION.to_string(),
        decision_id: "decision-1".to_string(),
        trace_id: None,
        session_id: None,
        on: HookCallback::HttpRequest,
        subject: json!({"request": {"host": "example.com"}}),
        preview: None,
        hashes: None,
        audit_context: None,
    };
    assert!(request.validate_semantics().is_ok());

    request.subject = json!(["not", "object"]);
    assert!(request
        .validate_semantics()
        .unwrap_err()
        .contains("subject"));

    request.subject = json!({});
    request.preview = Some(json!(null));
    assert!(request
        .validate_semantics()
        .unwrap_err()
        .contains("preview"));

    request.preview = Some(json!({}));
    request.hashes = Some(json!("sha256"));
    assert!(request.validate_semantics().unwrap_err().contains("hashes"));
}

#[test]
fn response_semantics_require_rewrite_fields_only_for_rewrite() {
    let mut response = HookDecisionResponse {
        decision: HookDecision::Rewrite,
        decision_id: Some("decision-1".to_string()),
        rule_id: None,
        priority: None,
        reason: None,
        ttl_ms: None,
        rewrite_target: None,
        rewrite_value: Some("[redacted]".to_string()),
        redactions: Vec::new(),
        audit_tags: Vec::new(),
    };
    assert!(response
        .validate_semantics()
        .unwrap_err()
        .contains("rewrite_target"));

    response.rewrite_target = Some("subject.path".to_string());
    assert!(response.validate_semantics().is_ok());

    response.decision = HookDecision::Block;
    assert!(response
        .validate_semantics()
        .unwrap_err()
        .contains("non-rewrite"));
}
