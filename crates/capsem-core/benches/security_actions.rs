//! Security action microbenchmarks.
//!
//! These benches keep the T6 security-event/action path measurable without
//! booting a VM or running a daemon. Regenerate with:
//! `cargo bench -p capsem-core --bench security_actions`.

use capsem_core::credential_broker::{
    broker_to_user_settings, CredentialObservation, CredentialProvider,
};
use capsem_core::net::ai_traffic::provider::ProviderKind;
use capsem_core::net::policy_config::{
    PolicyActionId, PolicyCallback, PolicyConfig, PolicyDecisionKind, PolicyRuleConfig,
};
use capsem_core::security_engine::{
    materialize_http_request_for_upstream, HttpRequestSecurityEvent, RuntimeSecurityEvent,
    SecurityActionRegistry, SecurityEvent,
};
use capsem_logger::{Decision, McpCall, ModelCall, NetEvent, WriteOp};
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::collections::BTreeMap;
use std::time::SystemTime;

const TEST_STORE_ENV: &str = "CAPSEM_CREDENTIAL_BROKER_TEST_STORE";

struct EnvVarGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let old = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, old }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

fn action_rule(actions: Vec<PolicyActionId>) -> PolicyRuleConfig {
    PolicyRuleConfig {
        on: PolicyCallback::HttpRequest,
        condition: "request.host == \"api.anthropic.com\"".to_string(),
        decision: PolicyDecisionKind::Action,
        priority: 0,
        reason: None,
        actions,
        rewrite_target: None,
        rewrite_value: None,
        strip_request_headers: Vec::new(),
        strip_response_headers: Vec::new(),
    }
}

fn decision_policy() -> PolicyConfig {
    let mut policy = PolicyConfig::default();
    policy.http.insert(
        "allow_anthropic".to_string(),
        PolicyRuleConfig {
            on: PolicyCallback::HttpRequest,
            condition: "request.host == \"api.anthropic.com\"".to_string(),
            decision: PolicyDecisionKind::Allow,
            priority: 10,
            reason: None,
            actions: Vec::new(),
            rewrite_target: None,
            rewrite_value: None,
            strip_request_headers: Vec::new(),
            strip_response_headers: Vec::new(),
        },
    );
    policy
}

fn brokered_header_event() -> (SecurityEvent, tempfile::TempDir, EnvVarGuard) {
    let tmp = tempfile::tempdir().unwrap();
    let store_path = tmp.path().join("broker-store.json");
    let guard = EnvVarGuard::set(TEST_STORE_ENV, store_path.as_os_str());
    let brokered = broker_to_user_settings(&CredentialObservation {
        provider: CredentialProvider::Anthropic,
        raw_value: "sk-ant-security-action-bench".to_string(),
        source: "http.request.headers.authorization".to_string(),
        event_type: Some("http.request".to_string()),
        confidence: 1.0,
        trace_id: None,
        context_json: None,
    })
    .unwrap();

    let mut headers = http::HeaderMap::new();
    headers.insert(
        http::header::AUTHORIZATION,
        http::HeaderValue::from_str(&brokered.credential_ref).unwrap(),
    );

    let event = SecurityEvent::new(PolicyCallback::HttpRequest).with_http_request(
        HttpRequestSecurityEvent::new(
            "api.anthropic.com",
            Some(ProviderKind::Anthropic),
            headers,
            None,
        ),
    );
    (event, tmp, guard)
}

fn net_write() -> WriteOp {
    WriteOp::NetEvent(NetEvent {
        event_id: None,
        timestamp: SystemTime::now(),
        domain: "api.anthropic.com".to_string(),
        port: 443,
        decision: Decision::Allowed,
        process_name: Some("bench".to_string()),
        pid: Some(42),
        method: Some("POST".to_string()),
        path: Some("/v1/messages".to_string()),
        query: None,
        status_code: Some(200),
        bytes_sent: 256,
        bytes_received: 512,
        duration_ms: 7,
        matched_rule: None,
        request_headers: None,
        response_headers: None,
        request_body_preview: None,
        response_body_preview: None,
        conn_type: Some("https".to_string()),
        policy_mode: None,
        policy_action: None,
        policy_rule: None,
        policy_reason: None,
        trace_id: Some("bench-trace".to_string()),
        credential_ref: Some(
            "credential:blake3:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .to_string(),
        ),
    })
}

fn model_write() -> WriteOp {
    WriteOp::ModelCall(ModelCall {
        event_id: None,
        timestamp: SystemTime::now(),
        provider: "anthropic".to_string(),
        model: Some("claude-bench".to_string()),
        process_name: Some("bench".to_string()),
        pid: Some(42),
        method: "POST".to_string(),
        path: "/v1/messages".to_string(),
        stream: false,
        system_prompt_preview: None,
        messages_count: 2,
        tools_count: 1,
        request_bytes: 256,
        request_body_preview: None,
        message_id: Some("msg_bench".to_string()),
        status_code: Some(200),
        text_content: Some("ok".to_string()),
        thinking_content: None,
        stop_reason: Some("end_turn".to_string()),
        input_tokens: Some(10),
        output_tokens: Some(2),
        usage_details: BTreeMap::new(),
        duration_ms: 12,
        response_bytes: 128,
        estimated_cost_usd: 0.0001,
        trace_id: Some("bench-trace".to_string()),
        credential_ref: None,
        tool_calls: Vec::new(),
        tool_responses: Vec::new(),
    })
}

fn mcp_write() -> WriteOp {
    WriteOp::McpCall(McpCall {
        event_id: None,
        timestamp: SystemTime::now(),
        server_name: "bench-server".to_string(),
        method: "tools/call".to_string(),
        tool_name: Some("bench_tool".to_string()),
        request_id: Some("1".to_string()),
        request_preview: Some("{\"x\":1}".to_string()),
        response_preview: Some("{\"ok\":true}".to_string()),
        decision: "allowed".to_string(),
        duration_ms: 3,
        error_message: None,
        process_name: Some("bench".to_string()),
        bytes_sent: 16,
        bytes_received: 16,
        policy_mode: None,
        policy_action: None,
        policy_rule: None,
        policy_reason: None,
        trace_id: Some("bench-trace".to_string()),
        credential_ref: None,
    })
}

fn bench_rule_match(c: &mut Criterion) {
    let policy = decision_policy();
    let subject = serde_json::json!({
        "request": {
            "host": "api.anthropic.com"
        }
    });

    c.bench_function("security_action_rule_match_noop", |b| {
        b.iter(|| {
            let matched = policy
                .find_matching_decision_rule(PolicyCallback::HttpRequest, black_box(&subject))
                .unwrap();
            black_box(matched);
        });
    });
}

fn bench_action_chain(c: &mut Criterion) {
    let registry = SecurityActionRegistry::with_builtin_actions();
    for (label, actions) in [
        (
            "security_action_chain_1",
            vec![PolicyActionId::CredentialBrokerCapture],
        ),
        (
            "security_action_chain_2",
            vec![
                PolicyActionId::CredentialBrokerCapture,
                PolicyActionId::CredentialBrokerSubstitute,
            ],
        ),
        (
            "security_action_chain_4",
            vec![
                PolicyActionId::CredentialBrokerCapture,
                PolicyActionId::CredentialBrokerSubstitute,
                PolicyActionId::CredentialBrokerCapture,
                PolicyActionId::CredentialBrokerSubstitute,
            ],
        ),
    ] {
        let rule = action_rule(actions);
        c.bench_function(label, |b| {
            b.iter(|| {
                let event = registry
                    .apply_rule_actions(
                        black_box(&rule),
                        SecurityEvent::new(PolicyCallback::HttpRequest),
                    )
                    .unwrap();
                black_box(event);
            });
        });
    }
}

fn bench_broker_substitute(c: &mut Criterion) {
    let registry = SecurityActionRegistry::with_builtin_actions();
    let rule = action_rule(vec![PolicyActionId::CredentialBrokerSubstitute]);
    let (event, _tmp, _guard) = brokered_header_event();

    c.bench_function("security_action_broker_substitute_header_ref", |b| {
        b.iter(|| {
            let event = registry
                .apply_rule_actions(black_box(&rule), black_box(event.clone()))
                .unwrap();
            let materialized = materialize_http_request_for_upstream(&event).unwrap();
            black_box(materialized);
        });
    });
}

fn bench_runtime_event_handoff(c: &mut Criterion) {
    let net = net_write();
    let model = model_write();
    let mcp = mcp_write();

    c.bench_function("security_event_runtime_classify_http", |b| {
        b.iter(|| {
            let event = RuntimeSecurityEvent::from_logger_write(black_box(net.clone()));
            black_box(event);
        });
    });

    c.bench_function("security_event_runtime_classify_model", |b| {
        b.iter(|| {
            let event = RuntimeSecurityEvent::from_logger_write(black_box(model.clone()));
            black_box(event);
        });
    });

    c.bench_function("security_event_runtime_classify_mcp", |b| {
        b.iter(|| {
            let event = RuntimeSecurityEvent::from_logger_write(black_box(mcp.clone()));
            black_box(event);
        });
    });
}

criterion_group!(
    benches,
    bench_rule_match,
    bench_action_chain,
    bench_broker_substitute,
    bench_runtime_event_handoff
);
criterion_main!(benches);
