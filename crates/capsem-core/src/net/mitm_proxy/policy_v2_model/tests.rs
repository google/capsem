use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use super::*;
use crate::net::policy_config::{PolicyRuleConfig, SettingsFile};
use crate::net::policy_confirm::{
    ConfirmArgs, Confirmer, ConfirmerKind, Decision as ConfirmDecision, PlaceholderConfirmer,
};

fn policy_from_toml(toml_text: &str) -> PolicyConfig {
    toml::from_str::<SettingsFile>(toml_text).unwrap().policy
}

fn headers(pairs: &[(&str, &str)]) -> http::HeaderMap {
    let mut headers = http::HeaderMap::new();
    for (name, value) in pairs {
        headers.insert(
            http::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
            http::HeaderValue::from_str(value).unwrap(),
        );
    }
    headers
}

async fn evaluate_model_request_policy_test(
    policy: &PolicyConfig,
    provider: ProviderKind,
    headers: &http::HeaderMap,
    body: &[u8],
) -> Option<ModelRequestPolicyOutcome> {
    let confirmer: Arc<dyn Confirmer> = Arc::new(PlaceholderConfirmer);
    let opts = crate::net::policy_confirm::default_confirm_backoff("confirm-model-test");
    evaluate_model_request_policy(policy, provider, headers, body, &confirmer, &opts).await
}

async fn evaluate_model_response_policy_test(
    policy: &PolicyConfig,
    provider: ProviderKind,
    request_meta: &RequestMeta,
    body: &[u8],
) -> Option<ModelResponsePolicyOutcome> {
    let confirmer: Arc<dyn Confirmer> = Arc::new(PlaceholderConfirmer);
    let opts = crate::net::policy_confirm::default_confirm_backoff("confirm-model-test");
    evaluate_model_response_policy(policy, provider, request_meta, body, &confirmer, &opts).await
}

struct MockConfirmer {
    decision: ConfirmDecision,
    calls: std::sync::Mutex<Vec<ConfirmArgs>>,
}

impl MockConfirmer {
    fn new(decision: ConfirmDecision) -> Arc<Self> {
        Arc::new(Self {
            decision,
            calls: std::sync::Mutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> Vec<ConfirmArgs> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl Confirmer for MockConfirmer {
    async fn confirm(&self, args: ConfirmArgs) -> ConfirmDecision {
        self.calls.lock().unwrap().push(args);
        self.decision
    }

    fn kind(&self) -> ConfirmerKind {
        ConfirmerKind::Automated
    }
}

fn openai_body(model: &str, secret: &str) -> String {
    format!(
        r#"{{"model":"{model}","messages":[{{"role":"system","content":"protect {secret}"}},{{"role":"user","content":"hello {secret}"}}],"tools":[{{"type":"function","function":{{"name":"lookup","parameters":{{"type":"object"}}}}}}]}}"#
    )
}

fn openai_tool_response_body(model: &str, call_id: &str, content: &str) -> String {
    format!(
        r#"{{"model":"{model}","messages":[{{"role":"user","content":"run lookup"}},{{"role":"assistant","tool_calls":[{{"id":"{call_id}","type":"function","function":{{"name":"lookup","arguments":"{{}}"}}}}]}},{{"role":"tool","tool_call_id":"{call_id}","content":"{content}"}}]}}"#
    )
}

fn openai_two_tool_response_body(
    model: &str,
    first_call_id: &str,
    first_content: &str,
    second_call_id: &str,
    second_content: &str,
) -> String {
    format!(
        r#"{{"model":"{model}","messages":[{{"role":"user","content":"run lookup"}},{{"role":"assistant","tool_calls":[{{"id":"{first_call_id}","type":"function","function":{{"name":"lookup","arguments":"{{}}"}}}},{{"id":"{second_call_id}","type":"function","function":{{"name":"lookup","arguments":"{{}}"}}}}]}},{{"role":"tool","tool_call_id":"{first_call_id}","content":"{first_content}"}},{{"role":"tool","tool_call_id":"{second_call_id}","content":"{second_content}"}}]}}"#
    )
}

fn openai_response_body(model: &str, content: &str) -> String {
    format!(
        r#"{{"id":"chatcmpl_resp","model":"{model}","choices":[{{"index":0,"message":{{"role":"assistant","content":"{content}"}},"finish_reason":"stop"}}]}}"#
    )
}

fn openai_tool_call_response_body(
    model: &str,
    call_id: &str,
    tool_name: &str,
    arguments: &str,
) -> String {
    let escaped_arguments = serde_json::to_string(arguments).unwrap();
    format!(
        r#"{{"id":"chatcmpl_tool","model":"{model}","choices":[{{"index":0,"message":{{"role":"assistant","content":null,"tool_calls":[{{"id":"{call_id}","type":"function","function":{{"name":"{tool_name}","arguments":{escaped_arguments}}}}}]}},"finish_reason":"tool_calls"}}]}}"#
    )
}

fn openai_two_tool_call_response_body(
    model: &str,
    first_call_id: &str,
    first_tool_name: &str,
    first_arguments: &str,
    second_call_id: &str,
    second_tool_name: &str,
    second_arguments: &str,
) -> String {
    let first_arguments = serde_json::to_string(first_arguments).unwrap();
    let second_arguments = serde_json::to_string(second_arguments).unwrap();
    format!(
        r#"{{"id":"chatcmpl_tool","model":"{model}","choices":[{{"index":0,"message":{{"role":"assistant","content":null,"tool_calls":[{{"id":"{first_call_id}","type":"function","function":{{"name":"{first_tool_name}","arguments":{first_arguments}}}}},{{"id":"{second_call_id}","type":"function","function":{{"name":"{second_tool_name}","arguments":{second_arguments}}}}}]}},"finish_reason":"tool_calls"}}]}}"#
    )
}

#[tokio::test]
async fn model_request_policy_matches_provider_model_counts_body_and_header() {
    let policy = policy_from_toml(
        r#"
[policy.model.allow_openai_with_header]
on = "model.request"
if = 'provider == "openai" && model == "gpt-4o" && messages_count == "2" && tools_count == "1" && has(messages) && has(request.headers.authorization) && request.headers.authorization.contains("Bearer") && request.body.contains("unit-secret")'
decision = "allow"
priority = 10
reason = "allow matched model request fields"
"#,
    );
    let headers = headers(&[("authorization", "Bearer test-token")]);
    let body = openai_body("gpt-4o", "unit-secret");

    let outcome = evaluate_model_request_policy_test(
        &policy,
        ProviderKind::OpenAi,
        &headers,
        body.as_bytes(),
    )
    .await
    .expect("rule should match");

    let ModelRequestPolicyOutcome::Continue(decision) = outcome else {
        panic!("allow rule should continue");
    };
    assert_eq!(decision.policy_mode.as_deref(), Some("enforce"));
    assert_eq!(decision.policy_action.as_deref(), Some("allow"));
    assert_eq!(
        decision.policy_rule.as_deref(),
        Some("policy.model.allow_openai_with_header")
    );
    assert_eq!(
        decision.policy_reason.as_deref(),
        Some("allow matched model request fields")
    );
}

#[tokio::test]
async fn model_request_policy_uses_truncated_json_model_fallback() {
    let policy = policy_from_toml(
        r#"
[policy.model.block_truncated]
on = "model.request"
if = 'provider == "openai" && model == "gpt-4o-mini" && request.body.contains("fallback-secret")'
decision = "block"
priority = 10
"#,
    );
    let body = br#"{"model":"gpt-4o-mini","messages":[{"role":"user","content":"fallback-secret"}"#;

    let outcome = evaluate_model_request_policy_test(
        &policy,
        ProviderKind::OpenAi,
        &http::HeaderMap::new(),
        body,
    )
    .await
    .expect("fallback model rule should match");

    let ModelRequestPolicyOutcome::Deny(decision) = outcome else {
        panic!("block rule should deny");
    };
    assert_eq!(decision.policy_action.as_deref(), Some("block"));
    assert_eq!(
        decision.policy_rule.as_deref(),
        Some("policy.model.block_truncated")
    );
}

#[tokio::test]
async fn model_request_policy_ask_accepts_with_placeholder_and_rewrite_redacts_body() {
    let ask_policy = policy_from_toml(
        r#"
[policy.model.ask_openai]
on = "model.request"
if = 'provider == "openai" && model == "gpt-4o"'
decision = "ask"
priority = 10
"#,
    );
    let body = openai_body("gpt-4o", "ask-secret");
    let ask = evaluate_model_request_policy_test(
        &ask_policy,
        ProviderKind::OpenAi,
        &http::HeaderMap::new(),
        body.as_bytes(),
    )
    .await
    .expect("ask rule should match");
    let ModelRequestPolicyOutcome::Continue(ask_decision) = ask else {
        panic!("placeholder-confirmed ask rule should continue");
    };
    assert_eq!(ask_decision.policy_action.as_deref(), Some("allow"));

    let rewrite_policy = policy_from_toml(
        r#"
[policy.model.rewrite_openai]
on = "model.request"
if = 'provider == "openai" && model == "gpt-4o"'
decision = "rewrite"
priority = 10
rewrite_target = 'request.body =~ "rewrite-(?P<suffix>[a-z]+)"'
rewrite_value = "[redacted-${suffix}]"
"#,
    );
    let rewrite = evaluate_model_request_policy_test(
        &rewrite_policy,
        ProviderKind::OpenAi,
        &http::HeaderMap::new(),
        openai_body("gpt-4o", "rewrite-token").as_bytes(),
    )
    .await
    .expect("rewrite rule should match");
    let ModelRequestPolicyOutcome::RewriteBody {
        decision: rewrite_decision,
        body: rewritten,
    } = rewrite
    else {
        panic!("model rewrite should return rewritten body");
    };
    let rewritten = String::from_utf8(rewritten).unwrap();
    assert!(
        rewritten.contains("[redacted-token]"),
        "rewritten body must contain the redacted capture, got: {rewritten}"
    );
    assert!(
        !rewritten.contains("rewrite-token"),
        "rewritten body must not contain the original secret, got: {rewritten}"
    );
    assert_eq!(rewrite_decision.policy_action.as_deref(), Some("rewrite"));
}

#[tokio::test]
async fn model_request_policy_rewrite_supports_canonical_request_data_target() {
    let rewrite_policy = policy_from_toml(
        r#"
[policy.model.rewrite_openai]
on = "model.request"
if = 'provider == "openai" && model == "gpt-4o" && request.data.contains("rewrite-token")'
decision = "rewrite"
priority = 10
rewrite_target = 'request.data =~ "rewrite-(?P<suffix>[a-z]+)"'
rewrite_value = "[redacted-${suffix}]"
"#,
    );
    let rewrite = evaluate_model_request_policy_test(
        &rewrite_policy,
        ProviderKind::OpenAi,
        &http::HeaderMap::new(),
        openai_body("gpt-4o", "rewrite-token").as_bytes(),
    )
    .await
    .expect("canonical rewrite rule should match");
    let ModelRequestPolicyOutcome::RewriteBody { body, .. } = rewrite else {
        panic!("canonical request.data rewrite should return rewritten body");
    };
    let rewritten = String::from_utf8(body).unwrap();
    assert!(rewritten.contains("[redacted-token]"));
    assert!(!rewritten.contains("rewrite-token"));
}

#[tokio::test]
async fn model_request_policy_rewrite_unsupported_target_fails_closed() {
    let rewrite_policy = policy_from_toml(
        r#"
[policy.model.unsupported_target]
on = "model.request"
if = 'provider == "openai" && model == "gpt-4o"'
decision = "rewrite"
priority = 10
rewrite_target = 'request.headers =~ "."'
rewrite_value = "x"
"#,
    );
    let outcome = evaluate_model_request_policy_test(
        &rewrite_policy,
        ProviderKind::OpenAi,
        &http::HeaderMap::new(),
        openai_body("gpt-4o", "ignored").as_bytes(),
    )
    .await
    .expect("rule should match");
    let ModelRequestPolicyOutcome::Deny(decision) = outcome else {
        panic!("unsupported rewrite target should fail closed");
    };
    assert_eq!(decision.policy_action.as_deref(), Some("rewrite"));
    assert!(decision
        .policy_reason
        .as_deref()
        .unwrap_or_default()
        .contains("unsupported model.request rewrite target"));
}

#[tokio::test]
async fn model_request_policy_rewrite_no_match_fails_closed() {
    let rewrite_policy = policy_from_toml(
        r#"
[policy.model.no_match]
on = "model.request"
if = 'provider == "openai" && model == "gpt-4o"'
decision = "rewrite"
priority = 10
rewrite_target = 'request.data =~ "this-string-is-not-in-the-body"'
rewrite_value = "x"
"#,
    );
    let outcome = evaluate_model_request_policy_test(
        &rewrite_policy,
        ProviderKind::OpenAi,
        &http::HeaderMap::new(),
        openai_body("gpt-4o", "real-content").as_bytes(),
    )
    .await
    .expect("rule should match");
    let ModelRequestPolicyOutcome::Deny(decision) = outcome else {
        panic!("non-matching rewrite should fail closed");
    };
    assert_eq!(decision.policy_action.as_deref(), Some("rewrite"));
    assert!(decision
        .policy_reason
        .as_deref()
        .unwrap_or_default()
        .contains("did not match model request body"));
}

#[tokio::test]
async fn model_request_policy_rewrite_non_utf8_body_fails_closed() {
    let rewrite_policy = policy_from_toml(
        r#"
[policy.model.binary_body]
on = "model.request"
if = 'provider == "openai"'
decision = "rewrite"
priority = 10
rewrite_target = 'request.data =~ "x"'
rewrite_value = "y"
"#,
    );
    let outcome = evaluate_model_request_policy_test(
        &rewrite_policy,
        ProviderKind::OpenAi,
        &http::HeaderMap::new(),
        &[0xFF, 0xFE, 0xFD, 0xFC],
    )
    .await
    .expect("rule should match by provider");
    let ModelRequestPolicyOutcome::Deny(decision) = outcome else {
        panic!("non-UTF-8 rewrite body should fail closed");
    };
    assert_eq!(decision.policy_action.as_deref(), Some("rewrite"));
    assert!(decision
        .policy_reason
        .as_deref()
        .unwrap_or_default()
        .contains("not UTF-8"));
}

#[tokio::test]
async fn model_request_policy_ask_deny_confirmer_blocks_and_redacts_snapshot() {
    let policy = policy_from_toml(
        r#"
[policy.model.ask_openai]
on = "model.request"
if = 'provider == "openai" && model == "gpt-4o" && request.body.contains("ask-secret")'
decision = "ask"
priority = 10
reason = "Ask before sending this model request"
"#,
    );
    let body = openai_body("gpt-4o", "ask-secret");
    let confirmer = MockConfirmer::new(ConfirmDecision::Deny);
    let confirmer_dyn = confirmer.clone() as Arc<dyn Confirmer>;
    let opts = crate::net::policy_confirm::default_confirm_backoff("confirm-model-test");

    let outcome = evaluate_model_request_policy(
        &policy,
        ProviderKind::OpenAi,
        &http::HeaderMap::new(),
        body.as_bytes(),
        &confirmer_dyn,
        &opts,
    )
    .await
    .expect("ask rule should match");

    let ModelRequestPolicyOutcome::Deny(decision) = outcome else {
        panic!("deny confirmer should block model request ask");
    };
    assert_eq!(decision.policy_action.as_deref(), Some("block"));
    assert_eq!(
        decision.policy_rule.as_deref(),
        Some("policy.model.ask_openai")
    );
    let calls = confirmer.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].rule_id, "security.rules.model.ask_openai");
    assert_eq!(
        calls[0].callback,
        crate::net::policy_config::PolicyCallback::ModelRequest
    );
    let snapshot = serde_json::to_string(&calls[0].args_snapshot).unwrap();
    assert!(
        !snapshot.contains("ask-secret"),
        "model confirm snapshots must not echo request body text: {snapshot}"
    );
}

#[tokio::test]
async fn model_request_policy_returns_none_when_no_rule_matches() {
    let policy = policy_from_toml(
        r#"
[policy.model.block_other_model]
on = "model.request"
if = 'provider == "openai" && model == "gpt-5"'
decision = "block"
priority = 10
"#,
    );
    let body = openai_body("gpt-4o", "safe");

    let outcome = evaluate_model_request_policy_test(
        &policy,
        ProviderKind::OpenAi,
        &http::HeaderMap::new(),
        body.as_bytes(),
    )
    .await;

    assert_eq!(outcome, None);
}

#[tokio::test]
async fn model_request_policy_invalid_runtime_condition_fails_closed() {
    let mut model = HashMap::new();
    model.insert(
        "bad_regex".to_string(),
        PolicyRuleConfig {
            on: PolicyCallback::ModelRequest,
            condition: "request.body.matches(\"[\")".to_string(),
            decision: PolicyDecisionKind::Allow,
            priority: 10,
            reason: None,
            rewrite_target: None,
            rewrite_value: None,
            strip_request_headers: Vec::new(),
            strip_response_headers: Vec::new(),
        },
    );
    let policy = PolicyConfig {
        model,
        ..PolicyConfig::default()
    };

    let outcome = evaluate_model_request_policy_test(
        &policy,
        ProviderKind::OpenAi,
        &http::HeaderMap::new(),
        openai_body("gpt-4o", "invalid-condition").as_bytes(),
    )
    .await
    .expect("invalid condition should fail closed");

    let ModelRequestPolicyOutcome::Deny(decision) = outcome else {
        panic!("invalid condition should deny");
    };
    assert_eq!(decision.policy_action.as_deref(), Some("block"));
    assert_eq!(
        decision.policy_rule.as_deref(),
        Some("policy.model.invalid_condition")
    );
}

#[tokio::test]
async fn model_tool_response_policy_blocks_secret_result_before_provider_dispatch() {
    let policy = policy_from_toml(
        r#"
[policy.model.block_secret_tool_result]
on = "model.tool_response"
if = 'provider == "openai" && model == "gpt-4o-mini" && tool.call_id == "call_secret" && content.contains("AWS_SECRET_ACCESS_KEY")'
decision = "block"
priority = 10
reason = "Do not send secret tool output to provider"
"#,
    );
    let body = openai_tool_response_body(
        "gpt-4o-mini",
        "call_secret",
        "AWS_SECRET_ACCESS_KEY=unit-secret",
    );

    let outcome = evaluate_model_request_policy_test(
        &policy,
        ProviderKind::OpenAi,
        &http::HeaderMap::new(),
        body.as_bytes(),
    )
    .await
    .expect("tool response rule should match");

    let ModelRequestPolicyOutcome::Deny(decision) = outcome else {
        panic!("secret tool response should deny before provider dispatch");
    };
    assert_eq!(decision.policy_mode.as_deref(), Some("enforce"));
    assert_eq!(decision.policy_action.as_deref(), Some("block"));
    assert_eq!(
        decision.policy_rule.as_deref(),
        Some("policy.model.block_secret_tool_result")
    );
    assert_eq!(
        decision.policy_reason.as_deref(),
        Some("Do not send secret tool output to provider")
    );
}

#[tokio::test]
async fn model_tool_response_policy_uses_global_priority_across_multiple_results() {
    let policy = policy_from_toml(
        r#"
[policy.model.allow_first_tool_result]
on = "model.tool_response"
if = 'provider == "openai" && tool.call_id == "call_safe"'
decision = "allow"
priority = 100
reason = "safe tool result"

[policy.model.block_second_tool_result_secret]
on = "model.tool_response"
if = 'provider == "openai" && content.contains("AWS_SECRET_ACCESS_KEY")'
decision = "block"
priority = 10
reason = "block later secret result"
"#,
    );
    let body = openai_two_tool_response_body(
        "gpt-4o-mini",
        "call_secret",
        "AWS_SECRET_ACCESS_KEY=unit-secret",
        "call_safe",
        "safe output",
    );

    let outcome = evaluate_model_request_policy_test(
        &policy,
        ProviderKind::OpenAi,
        &http::HeaderMap::new(),
        body.as_bytes(),
    )
    .await
    .expect("later higher-priority tool response rule should match");

    let ModelRequestPolicyOutcome::Deny(decision) = outcome else {
        panic!("highest-priority matching tool response rule should deny");
    };
    assert_eq!(decision.policy_action.as_deref(), Some("block"));
    assert_eq!(
        decision.policy_rule.as_deref(),
        Some("policy.model.block_second_tool_result_secret")
    );
}

#[tokio::test]
async fn model_tool_response_policy_does_not_let_one_allowed_result_bypass_another_block() {
    let policy = policy_from_toml(
        r#"
[policy.model.allow_safe_tool_result]
on = "model.tool_response"
if = 'provider == "openai" && tool.call_id == "call_safe"'
decision = "allow"
priority = 1
reason = "safe tool result"

[policy.model.block_any_secret_tool_result]
on = "model.tool_response"
if = 'provider == "openai" && content.contains("AWS_SECRET_ACCESS_KEY")'
decision = "block"
priority = 100
reason = "block any secret result"
"#,
    );
    let body = openai_two_tool_response_body(
        "gpt-4o-mini",
        "call_secret",
        "AWS_SECRET_ACCESS_KEY=unit-secret",
        "call_safe",
        "safe output",
    );

    let outcome = evaluate_model_request_policy_test(
        &policy,
        ProviderKind::OpenAi,
        &http::HeaderMap::new(),
        body.as_bytes(),
    )
    .await
    .expect("secret tool response rule should still deny");

    let ModelRequestPolicyOutcome::Deny(decision) = outcome else {
        panic!("an allow decision for one tool response must not allow a separate secret result");
    };
    assert_eq!(decision.policy_action.as_deref(), Some("block"));
    assert_eq!(
        decision.policy_rule.as_deref(),
        Some("policy.model.block_any_secret_tool_result")
    );
}

#[tokio::test]
async fn model_tool_response_policy_rewrites_secret_result_body() {
    let policy = policy_from_toml(
        r#"
[policy.model.rewrite_secret_tool_result]
on = "model.tool_response"
if = 'provider == "openai" && model == "gpt-4o-mini" && content.contains("AWS_SECRET_ACCESS_KEY")'
decision = "rewrite"
priority = 10
reason = "Redact secret tool output before provider dispatch"
rewrite_target = 'content =~ "AWS_SECRET_ACCESS_KEY=[^\\s\"]+"'
rewrite_value = "AWS_SECRET_ACCESS_KEY=[redacted]"
"#,
    );
    let body = openai_tool_response_body(
        "gpt-4o-mini",
        "call_secret",
        "prefix AWS_SECRET_ACCESS_KEY=unit-secret suffix",
    );

    let outcome = evaluate_model_request_policy_test(
        &policy,
        ProviderKind::OpenAi,
        &http::HeaderMap::new(),
        body.as_bytes(),
    )
    .await
    .expect("tool response rewrite rule should match");

    let ModelRequestPolicyOutcome::RewriteBody {
        decision,
        body: rewritten,
    } = outcome
    else {
        panic!("secret tool response should rewrite request body");
    };
    assert_eq!(decision.policy_mode.as_deref(), Some("enforce"));
    assert_eq!(decision.policy_action.as_deref(), Some("rewrite"));
    assert_eq!(
        decision.policy_rule.as_deref(),
        Some("policy.model.rewrite_secret_tool_result")
    );
    let rewritten = String::from_utf8(rewritten).expect("rewritten body should stay UTF-8");
    assert!(rewritten.contains("AWS_SECRET_ACCESS_KEY=[redacted]"));
    assert!(!rewritten.contains("unit-secret"));
}

#[tokio::test]
async fn model_response_policy_blocks_secret_text_before_guest_delivery() {
    let policy = policy_from_toml(
        r#"
[policy.model.block_secret_response]
on = "model.response"
if = 'provider == "openai" && model == "gpt-4o-mini" && response.text.contains("response-secret")'
decision = "block"
priority = 10
reason = "Do not show secret model text"
"#,
    );
    let request_meta = request_parser::parse_request(
        ProviderKind::OpenAi,
        openai_body("gpt-4o-mini", "safe").as_bytes(),
    );
    let response = openai_response_body("gpt-4o-mini", "hello response-secret");

    let outcome = evaluate_model_response_policy_test(
        &policy,
        ProviderKind::OpenAi,
        &request_meta,
        response.as_bytes(),
    )
    .await
    .expect("model response rule should match");

    let ModelResponsePolicyOutcome::Deny(decision) = outcome else {
        panic!("secret model response should deny before guest delivery");
    };
    assert_eq!(decision.policy_mode.as_deref(), Some("enforce"));
    assert_eq!(decision.policy_action.as_deref(), Some("block"));
    assert_eq!(
        decision.policy_rule.as_deref(),
        Some("policy.model.block_secret_response")
    );
}

#[tokio::test]
async fn model_response_policy_rewrites_secret_text_body() {
    let policy = policy_from_toml(
        r#"
[policy.model.rewrite_secret_response]
on = "model.response"
if = 'provider == "openai" && response.text.contains("response-secret")'
decision = "rewrite"
priority = 10
reason = "Redact secret model text"
rewrite_target = 'response.text =~ "response-secret"'
rewrite_value = "[redacted-response]"
"#,
    );
    let request_meta = request_parser::parse_request(
        ProviderKind::OpenAi,
        openai_body("gpt-4o-mini", "safe").as_bytes(),
    );
    let response = openai_response_body("gpt-4o-mini", "hello response-secret");

    let outcome = evaluate_model_response_policy_test(
        &policy,
        ProviderKind::OpenAi,
        &request_meta,
        response.as_bytes(),
    )
    .await
    .expect("model response rewrite rule should match");

    let ModelResponsePolicyOutcome::RewriteBody {
        decision,
        body: rewritten,
    } = outcome
    else {
        panic!("secret model response should rewrite body before guest delivery");
    };
    assert_eq!(decision.policy_action.as_deref(), Some("rewrite"));
    let rewritten = String::from_utf8(rewritten).expect("rewritten body should be UTF-8");
    assert!(rewritten.contains("[redacted-response]"));
    assert!(!rewritten.contains("response-secret"));
}

#[tokio::test]
async fn model_tool_call_policy_blocks_provider_emitted_call_before_guest_delivery() {
    let policy = policy_from_toml(
        r#"
[policy.model.block_secret_tool_call]
on = "model.tool_call"
if = 'provider == "openai" && model == "gpt-4o-mini" && tool.name == "leak_secret" && tool.arguments.secret.contains("tool-call-secret")'
decision = "block"
priority = 10
reason = "Do not let model request secret-leaking tool"
"#,
    );
    let request_meta = request_parser::parse_request(
        ProviderKind::OpenAi,
        openai_body("gpt-4o-mini", "safe").as_bytes(),
    );
    let response = openai_tool_call_response_body(
        "gpt-4o-mini",
        "call_secret",
        "leak_secret",
        r#"{"secret":"tool-call-secret"}"#,
    );

    let outcome = evaluate_model_response_policy_test(
        &policy,
        ProviderKind::OpenAi,
        &request_meta,
        response.as_bytes(),
    )
    .await
    .expect("model tool-call rule should match");

    let ModelResponsePolicyOutcome::Deny(decision) = outcome else {
        panic!("unsafe tool call should deny before guest delivery");
    };
    assert_eq!(decision.policy_action.as_deref(), Some("block"));
    assert_eq!(
        decision.policy_rule.as_deref(),
        Some("policy.model.block_secret_tool_call")
    );
}

#[tokio::test]
async fn model_tool_call_policy_ask_deny_confirmer_blocks_and_redacts_snapshot() {
    let policy = policy_from_toml(
        r#"
[policy.model.ask_secret_tool_call]
on = "model.tool_call"
if = 'provider == "openai" && tool.name == "leak_secret" && tool.arguments.secret.contains("tool-call-secret")'
decision = "ask"
priority = 10
reason = "Ask before delivering model tool calls"
"#,
    );
    let request_meta = request_parser::parse_request(
        ProviderKind::OpenAi,
        openai_body("gpt-4o-mini", "safe").as_bytes(),
    );
    let response = openai_tool_call_response_body(
        "gpt-4o-mini",
        "call_secret",
        "leak_secret",
        r#"{"secret":"tool-call-secret"}"#,
    );
    let confirmer = MockConfirmer::new(ConfirmDecision::Deny);
    let confirmer_dyn = confirmer.clone() as Arc<dyn Confirmer>;
    let opts = crate::net::policy_confirm::default_confirm_backoff("confirm-model-test");

    let outcome = evaluate_model_response_policy(
        &policy,
        ProviderKind::OpenAi,
        &request_meta,
        response.as_bytes(),
        &confirmer_dyn,
        &opts,
    )
    .await
    .expect("model tool-call ask rule should match");

    let ModelResponsePolicyOutcome::Deny(decision) = outcome else {
        panic!("deny confirmer should block model tool-call ask");
    };
    assert_eq!(decision.policy_action.as_deref(), Some("block"));
    assert_eq!(
        decision.policy_rule.as_deref(),
        Some("policy.model.ask_secret_tool_call")
    );
    let calls = confirmer.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].rule_id,
        "security.rules.model.ask_secret_tool_call"
    );
    assert_eq!(
        calls[0].callback,
        crate::net::policy_config::PolicyCallback::ModelToolCall
    );
    let snapshot = serde_json::to_string(&calls[0].args_snapshot).unwrap();
    assert!(
        !snapshot.contains("tool-call-secret"),
        "model tool-call confirm snapshots must not echo arguments: {snapshot}"
    );
}

#[tokio::test]
async fn model_tool_call_policy_does_not_let_one_allowed_call_bypass_another_block() {
    let policy = policy_from_toml(
        r#"
[policy.model.allow_safe_tool_call]
on = "model.tool_call"
if = 'provider == "openai" && tool.name == "safe_lookup"'
decision = "allow"
priority = 1
reason = "safe call"

[policy.model.block_secret_tool_call]
on = "model.tool_call"
if = 'provider == "openai" && tool.arguments.secret.contains("tool-call-secret")'
decision = "block"
priority = 100
reason = "secret call"
"#,
    );
    let request_meta = request_parser::parse_request(
        ProviderKind::OpenAi,
        openai_body("gpt-4o-mini", "safe").as_bytes(),
    );
    let response = openai_two_tool_call_response_body(
        "gpt-4o-mini",
        "call_secret",
        "leak_secret",
        r#"{"secret":"tool-call-secret"}"#,
        "call_safe",
        "safe_lookup",
        r#"{"city":"NYC"}"#,
    );

    let outcome = evaluate_model_response_policy_test(
        &policy,
        ProviderKind::OpenAi,
        &request_meta,
        response.as_bytes(),
    )
    .await
    .expect("unsafe sibling tool-call rule should match");

    let ModelResponsePolicyOutcome::Deny(decision) = outcome else {
        panic!("an allow for one tool call must not allow a separate unsafe call");
    };
    assert_eq!(decision.policy_action.as_deref(), Some("block"));
    assert_eq!(
        decision.policy_rule.as_deref(),
        Some("policy.model.block_secret_tool_call")
    );
}

#[tokio::test]
async fn model_tool_call_policy_rewrites_provider_emitted_arguments() {
    let policy = policy_from_toml(
        r#"
[policy.model.rewrite_secret_tool_call]
on = "model.tool_call"
if = 'provider == "openai" && tool.name == "leak_secret" && tool.arguments.secret.contains("tool-call-secret")'
decision = "rewrite"
priority = 10
reason = "Redact model-emitted tool arguments"
rewrite_target = 'tool.arguments =~ "tool-call-secret"'
rewrite_value = "[redacted-tool-call]"
"#,
    );
    let request_meta = request_parser::parse_request(
        ProviderKind::OpenAi,
        openai_body("gpt-4o-mini", "safe").as_bytes(),
    );
    let response = openai_tool_call_response_body(
        "gpt-4o-mini",
        "call_secret",
        "leak_secret",
        r#"{"secret":"tool-call-secret"}"#,
    );

    let outcome = evaluate_model_response_policy_test(
        &policy,
        ProviderKind::OpenAi,
        &request_meta,
        response.as_bytes(),
    )
    .await
    .expect("model tool-call rewrite rule should match");

    let ModelResponsePolicyOutcome::RewriteBody {
        decision,
        body: rewritten,
    } = outcome
    else {
        panic!("unsafe tool call should rewrite before guest delivery");
    };
    assert_eq!(decision.policy_action.as_deref(), Some("rewrite"));
    let rewritten = String::from_utf8(rewritten).expect("rewritten body should be UTF-8");
    assert!(rewritten.contains("[redacted-tool-call]"));
    assert!(!rewritten.contains("tool-call-secret"));
}
