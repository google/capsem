//! Tests for `policy_hook`.

use super::*;
use serde_json::json;
use std::sync::{Arc, Mutex};

fn sample_request() -> HookDecisionRequest {
    HookDecisionRequest {
        spec_version: POLICY_HOOK_SPEC_VERSION.to_string(),
        decision_id: "decision-1".to_string(),
        trace_id: Some("trace-1".to_string()),
        session_id: Some("vm-1".to_string()),
        on: crate::net::policy_hook_spec::HookCallback::ModelToolCall,
        subject: json!({"tool_name": "fetch_secret", "arguments": {"path": "/tmp/key"}}),
        preview: None,
        hashes: None,
        audit_context: None,
    }
}

#[test]
fn endpoint_config_defaults_and_rejects_unknown_fields() {
    let config: PolicyHookEndpointConfig = serde_json::from_value(json!({
        "id": "fixture",
        "decision_url": "https://hooks.example.com/v1/policy/decision"
    }))
    .unwrap();
    assert_eq!(config.timeout_ms, 2_000);
    assert_eq!(config.body_cap_bytes, 256 * 1024);
    assert!(config.allow_insecure_localhost);
    assert_eq!(config.fail_closed_decision, HookDecision::Block);

    let err = serde_json::from_value::<PolicyHookEndpointConfig>(json!({
        "id": "fixture",
        "decision_url": "https://hooks.example.com/v1/policy/decision",
        "surprise": true
    }))
    .unwrap_err();
    assert!(err.to_string().contains("unknown field"));
}

#[test]
fn endpoint_config_feeds_runtime_endpoint() {
    let endpoint = PolicyHookEndpoint::from_config(PolicyHookEndpointConfig {
        id: "corp".to_string(),
        decision_url: "https://hooks.example.com/v1/policy/decision".to_string(),
        bearer_token: Some("token".to_string()),
        timeout_ms: 500,
        body_cap_bytes: 4096,
        allow_insecure_localhost: false,
        fail_closed_decision: HookDecision::Ask,
    });
    assert_eq!(endpoint.id, "corp");
    assert_eq!(endpoint.bearer_token.as_deref(), Some("token"));
    assert_eq!(endpoint.timeout_ms, 500);
    assert_eq!(endpoint.body_cap_bytes, 4096);
    assert!(!endpoint.allow_insecure_localhost);
    assert_eq!(endpoint.fail_closed_decision, HookDecision::Ask);
}

#[derive(Debug, Clone)]
struct SeenRequest {
    url: String,
    bearer_token: Option<String>,
    body: String,
}

struct FixtureTransport {
    status: u16,
    body: Vec<u8>,
    seen: Mutex<Option<SeenRequest>>,
}

impl FixtureTransport {
    fn new(status: u16, body: impl AsRef<[u8]>) -> Arc<Self> {
        Arc::new(Self {
            status,
            body: body.as_ref().to_vec(),
            seen: Mutex::new(None),
        })
    }
}

impl PolicyHookTransport for FixtureTransport {
    fn post<'a>(
        &'a self,
        url: reqwest::Url,
        _timeout: std::time::Duration,
        bearer_token: Option<String>,
        body: Vec<u8>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<PolicyHookHttpResponse, PolicyHookError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            *self.seen.lock().unwrap() = Some(SeenRequest {
                url: url.to_string(),
                bearer_token,
                body: String::from_utf8(body).unwrap(),
            });
            Ok(PolicyHookHttpResponse {
                status: self.status,
                body: self.body.clone(),
            })
        })
    }
}

#[tokio::test]
async fn local_http_hook_returns_decision_and_records_audit() {
    let transport = FixtureTransport::new(
        200,
        r#"{"decision":"rewrite","decision_id":"decision-1","rule_id":"hook.fixture","rewrite_target":"subject.arguments.path","rewrite_value":"[redacted]","audit_tags":["fixture"]}"#,
    );
    let endpoint = PolicyHookEndpoint::new("fixture", "http://127.0.0.1/v1/policy/decision");
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("hook.db");

    {
        let writer = DbWriter::open(&db_path, 8).unwrap();
        let outcome = PolicyHookClient::with_transport(transport.clone())
            .decide(&endpoint, &sample_request(), Some(&writer))
            .await;
        assert!(!outcome.failed_closed);
        assert_eq!(outcome.response.decision, HookDecision::Rewrite);
        assert_eq!(
            outcome.response.rewrite_value.as_deref(),
            Some("[redacted]")
        );
    }

    let seen = transport.seen.lock().unwrap().clone().unwrap();
    assert_eq!(seen.url, "http://127.0.0.1/v1/policy/decision");
    assert!(seen.bearer_token.is_none());
    assert!(seen.body.contains("\"on\":\"model.tool_call\""));

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let (status, decision, callback, tags): (String, Option<String>, String, Option<String>) = conn
        .query_row(
            "SELECT status, decision, callback, audit_tags FROM policy_hook_events",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(status, "allowed");
    assert_eq!(decision.as_deref(), Some("rewrite"));
    assert_eq!(callback, "model.tool_call");
    assert_eq!(tags.as_deref(), Some(r#"["fixture"]"#));
}

#[tokio::test]
async fn nonlocal_http_endpoint_fails_closed_before_dial() {
    let endpoint = PolicyHookEndpoint::new("corp", "http://example.com/v1/policy/decision");
    let outcome = PolicyHookClient::new()
        .decide(&endpoint, &sample_request(), None)
        .await;

    assert!(outcome.failed_closed);
    assert_eq!(outcome.response.decision, HookDecision::Block);
    assert!(outcome
        .error
        .as_deref()
        .unwrap_or_default()
        .contains("HTTPS"));
}

#[tokio::test]
async fn malformed_schema_fails_closed_and_records_error() {
    let transport = FixtureTransport::new(200, r#"{"decision":"warn"}"#);
    let endpoint = PolicyHookEndpoint::new("fixture", "http://127.0.0.1/v1/policy/decision");
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("hook_error.db");

    {
        let writer = DbWriter::open(&db_path, 8).unwrap();
        let outcome = PolicyHookClient::with_transport(transport)
            .decide(&endpoint, &sample_request(), Some(&writer))
            .await;
        assert!(outcome.failed_closed);
        assert_eq!(outcome.response.decision, HookDecision::Block);
        assert!(outcome
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("schema"));
    }

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let (status, fallback, error): (String, Option<String>, Option<String>) = conn
        .query_row(
            "SELECT status, fallback, error FROM policy_hook_events",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(status, "error");
    assert_eq!(fallback.as_deref(), Some("block"));
    assert!(error.as_deref().unwrap_or_default().contains("schema"));
}

#[tokio::test]
async fn request_body_cap_fails_closed_without_network() {
    let mut endpoint = PolicyHookEndpoint::new("fixture", "http://127.0.0.1:1/v1/policy/decision");
    endpoint.body_cap_bytes = 8;
    let outcome = PolicyHookClient::new()
        .decide(&endpoint, &sample_request(), None)
        .await;

    assert!(outcome.failed_closed);
    assert_eq!(outcome.response.decision, HookDecision::Block);
    assert!(outcome
        .error
        .as_deref()
        .unwrap_or_default()
        .contains("exceeds cap"));
}
