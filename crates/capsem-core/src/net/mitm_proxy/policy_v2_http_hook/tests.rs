use std::sync::Arc;

use async_trait::async_trait;

use crate::net::mitm_proxy::hooks::{ConnMeta, HookState};
use crate::net::mitm_proxy::pipeline::{DispatchOutcome, Pipeline};
use crate::net::mitm_proxy::protocol::Protocol;
use crate::net::policy_config::SettingsFile;
use crate::net::policy_confirm::{
    ConfirmArgs, Confirmer, ConfirmerKind, Decision as ConfirmDecision,
};

use super::*;

fn pipeline_for(toml_text: &str) -> Pipeline {
    let settings: SettingsFile = toml::from_str(toml_text).unwrap();
    let policy = Arc::new(tokio::sync::RwLock::new(Arc::new(settings.policy)));
    Pipeline::builder()
        .register(Arc::new(PolicyV2HttpHook::new(policy)))
        .build()
}

fn pipeline_for_confirmer(toml_text: &str, confirmer: Arc<dyn Confirmer>) -> Pipeline {
    let settings: SettingsFile = toml::from_str(toml_text).unwrap();
    let policy = Arc::new(tokio::sync::RwLock::new(Arc::new(settings.policy)));
    Pipeline::builder()
        .register(Arc::new(
            PolicyV2HttpHook::new(policy).with_confirmer(confirmer),
        ))
        .build()
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

fn request_parts() -> http::request::Parts {
    let request = http::Request::builder()
        .method("GET")
        .uri("/openai/capsem?token=secret")
        .header("host", "github.com")
        .header("authorization", "Bearer secret")
        .body(())
        .unwrap();
    request.into_parts().0
}

fn response_parts() -> http::response::Parts {
    let response = http::Response::builder()
        .status(302)
        .header("location", "https://github.com/openai/capsem?ref=secret")
        .header("set-cookie", "session=secret")
        .header("x-secret-token", "secret")
        .body(())
        .unwrap();
    response.into_parts().0
}

fn conn() -> ConnMeta {
    ConnMeta {
        domain: "github.com".to_string(),
        process_name: Some("agent".to_string()),
        port: 443,
        protocol: Protocol::Tls,
        ai_provider: None,
    }
}

#[tokio::test]
async fn http_policy_v2_block_stops_before_upstream() {
    let pipeline = pipeline_for(
        r#"
[policy.http.block_openai_github]
on = "http.request"
if = 'request.host == "github.com" && request.path.matches("^/openai(/|$)")'
decision = "block"
priority = 10
reason = "Do not fetch OpenAI-owned GitHub code"
"#,
    );
    let mut parts = request_parts();
    let mut state = HookState::default();

    let outcome = pipeline
        .dispatch(Event::RawRequestHead(&mut parts), &mut state, None, &conn())
        .await;

    assert!(matches!(outcome, DispatchOutcome::Stopped(_)));
    let decision = state
        .peek::<LastHttpPolicyV2Decision>()
        .expect("Policy V2 HTTP decision should be stashed");
    assert_eq!(decision.policy_mode.as_deref(), Some("enforce"));
    assert_eq!(decision.policy_action.as_deref(), Some("block"));
    assert_eq!(
        decision.policy_rule.as_deref(),
        Some("policy.http.block_openai_github")
    );
    assert_eq!(
        decision.policy_reason.as_deref(),
        Some("Do not fetch OpenAI-owned GitHub code")
    );
}

#[tokio::test]
async fn http_policy_v2_request_ask_accept_confirmer_continues() {
    let confirmer = MockConfirmer::new(ConfirmDecision::Accept);
    let pipeline = pipeline_for_confirmer(
        r#"
[policy.http.ask_openai_github]
on = "http.request"
if = 'request.host == "github.com" && request.path.matches("^/openai(/|$)")'
decision = "ask"
priority = 10
reason = "Ask before fetching OpenAI-owned GitHub code"
"#,
        confirmer.clone() as Arc<dyn Confirmer>,
    );
    let mut parts = request_parts();
    let mut state = HookState::default();

    let outcome = pipeline
        .dispatch(Event::RawRequestHead(&mut parts), &mut state, None, &conn())
        .await;

    assert!(matches!(outcome, DispatchOutcome::Completed));
    let decision = state
        .peek::<LastHttpPolicyV2Decision>()
        .expect("Policy V2 HTTP decision should be stashed");
    assert_eq!(decision.policy_action.as_deref(), Some("allow"));
    assert_eq!(
        decision.policy_rule.as_deref(),
        Some("policy.http.ask_openai_github")
    );
    let calls = confirmer.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].rule_id, "security.rules.http.ask_openai_github");
    assert_eq!(
        calls[0].callback,
        crate::net::policy_config::PolicyCallback::HttpRequest
    );
    assert_eq!(
        calls[0]
            .args_snapshot
            .get("request")
            .and_then(|value| value.get("path")),
        Some(&serde_json::json!("/openai/capsem"))
    );
    let snapshot = serde_json::to_string(&calls[0].args_snapshot).unwrap();
    assert!(
        !snapshot.contains("Bearer secret") && !snapshot.contains("authorization"),
        "HTTP confirm snapshots must not expose request headers: {snapshot}"
    );
}

#[tokio::test]
async fn http_policy_v2_request_ask_deny_confirmer_blocks() {
    let confirmer = MockConfirmer::new(ConfirmDecision::Deny);
    let pipeline = pipeline_for_confirmer(
        r#"
[policy.http.ask_openai_github]
on = "http.request"
if = 'request.host == "github.com" && request.path.matches("^/openai(/|$)")'
decision = "ask"
priority = 10
reason = "Ask before fetching OpenAI-owned GitHub code"
"#,
        confirmer.clone() as Arc<dyn Confirmer>,
    );
    let mut parts = request_parts();
    let mut state = HookState::default();

    let outcome = pipeline
        .dispatch(Event::RawRequestHead(&mut parts), &mut state, None, &conn())
        .await;

    assert!(matches!(outcome, DispatchOutcome::Stopped(_)));
    let decision = state
        .peek::<LastHttpPolicyV2Decision>()
        .expect("Policy V2 HTTP decision should be stashed");
    assert_eq!(decision.policy_action.as_deref(), Some("block"));
    assert_eq!(
        decision.policy_rule.as_deref(),
        Some("policy.http.ask_openai_github")
    );
    assert_eq!(confirmer.calls().len(), 1);
}

#[tokio::test]
async fn http_policy_v2_rewrite_strips_headers_and_mutates_path() {
    let pipeline = pipeline_for(
        r#"
[policy.http.rewrite_openai_github]
on = "http.request"
if = 'request.host == "github.com" && request.path.matches("^/openai/") && has(request.headers.authorization)'
decision = "rewrite"
priority = 10
reason = "Route through the allowed mirror and remove credentials"
rewrite_target = 'request.url =~ "^https://github\.com/openai/(?P<repo>[^/?#]+)(?P<rest>.*)$"'
rewrite_value = "https://github.com/openclaw/${repo}${rest}"
strip_request_headers = ["authorization"]
"#,
    );
    let mut parts = request_parts();
    let mut state = HookState::default();

    let outcome = pipeline
        .dispatch(Event::RawRequestHead(&mut parts), &mut state, None, &conn())
        .await;

    assert!(matches!(outcome, DispatchOutcome::Completed));
    assert_eq!(
        parts.uri.path_and_query().map(|value| value.as_str()),
        Some("/openclaw/capsem?token=secret")
    );
    assert!(
        !parts.headers.contains_key("authorization"),
        "credential header must be stripped before upstream dispatch"
    );
    let decision = state
        .peek::<LastHttpPolicyV2Decision>()
        .expect("Policy V2 HTTP rewrite decision should be stashed");
    assert_eq!(decision.policy_action.as_deref(), Some("rewrite"));
    assert_eq!(
        decision.policy_rule.as_deref(),
        Some("policy.http.rewrite_openai_github")
    );
}

#[tokio::test]
async fn http_policy_v2_rewrite_rejects_cross_host_url_rewrites() {
    let pipeline = pipeline_for(
        r#"
[policy.http.rewrite_to_other_host]
on = "http.request"
if = 'request.host == "github.com" && request.path.matches("^/openai/")'
decision = "rewrite"
priority = 10
rewrite_target = 'request.url =~ "^https://github\.com/openai/.*$"'
rewrite_value = "https://evil.example/stolen"
"#,
    );
    let mut parts = request_parts();
    let original_uri = parts.uri.clone();
    let mut state = HookState::default();

    let outcome = pipeline
        .dispatch(Event::RawRequestHead(&mut parts), &mut state, None, &conn())
        .await;

    assert!(matches!(outcome, DispatchOutcome::Stopped(_)));
    assert_eq!(
        parts.uri, original_uri,
        "failed host-changing rewrites must not mutate the request head"
    );
    let decision = state
        .peek::<LastHttpPolicyV2Decision>()
        .expect("Policy V2 HTTP rewrite decision should be stashed");
    assert_eq!(decision.policy_action.as_deref(), Some("rewrite"));
    assert!(decision
        .policy_reason
        .as_deref()
        .is_some_and(|reason| reason.contains("cannot change upstream host")));
}

#[tokio::test]
async fn http_policy_v2_response_rewrite_strips_secret_headers() {
    let pipeline = pipeline_for(
        r#"
[policy.http.strip_response_credentials]
on = "http.response"
if = 'response.status == "302"'
decision = "rewrite"
priority = 10
reason = "Do not return upstream credentials to the guest"
strip_response_headers = ["Set-Cookie", "X-Secret-Token"]
"#,
    );
    let mut parts = response_parts();
    let mut state = HookState::default();

    let outcome = pipeline
        .dispatch(
            Event::RawResponseHead(&mut parts),
            &mut state,
            None,
            &conn(),
        )
        .await;

    assert!(matches!(outcome, DispatchOutcome::Completed));
    assert!(
        !parts.headers.contains_key("set-cookie"),
        "credential response header must be stripped before guest delivery"
    );
    assert!(
        !parts.headers.contains_key("x-secret-token"),
        "secret response header must be stripped before telemetry capture"
    );
    assert!(
        parts.headers.contains_key("location"),
        "unlisted response headers must be preserved"
    );
    let decision = state
        .peek::<LastHttpPolicyV2Decision>()
        .expect("Policy V2 HTTP response rewrite decision should be stashed");
    assert_eq!(decision.policy_action.as_deref(), Some("rewrite"));
    assert_eq!(
        decision.policy_rule.as_deref(),
        Some("policy.http.strip_response_credentials")
    );
}

#[tokio::test]
async fn http_policy_v2_response_ask_confirmer_resolves() {
    let toml = r#"
[policy.http.ask_redirect]
on = "http.response"
if = 'response.status == "302"'
decision = "ask"
priority = 10
reason = "Ask before returning redirects"
"#;

    let accept_confirmer = MockConfirmer::new(ConfirmDecision::Accept);
    let accept_pipeline =
        pipeline_for_confirmer(toml, accept_confirmer.clone() as Arc<dyn Confirmer>);
    let mut accept_parts = response_parts();
    let mut accept_state = HookState::default();
    let outcome = accept_pipeline
        .dispatch(
            Event::RawResponseHead(&mut accept_parts),
            &mut accept_state,
            None,
            &conn(),
        )
        .await;
    assert!(matches!(outcome, DispatchOutcome::Completed));
    assert_eq!(
        accept_state
            .peek::<LastHttpPolicyV2Decision>()
            .and_then(|decision| decision.policy_action.as_deref()),
        Some("allow")
    );
    assert_eq!(
        accept_confirmer.calls()[0].callback,
        crate::net::policy_config::PolicyCallback::HttpResponse
    );
    assert_eq!(
        accept_confirmer.calls()[0]
            .args_snapshot
            .get("response")
            .and_then(|value| value.get("status")),
        Some(&serde_json::json!("302"))
    );

    let deny_confirmer = MockConfirmer::new(ConfirmDecision::Deny);
    let deny_pipeline = pipeline_for_confirmer(toml, deny_confirmer.clone() as Arc<dyn Confirmer>);
    let mut deny_parts = response_parts();
    let mut deny_state = HookState::default();
    let outcome = deny_pipeline
        .dispatch(
            Event::RawResponseHead(&mut deny_parts),
            &mut deny_state,
            None,
            &conn(),
        )
        .await;
    assert!(matches!(outcome, DispatchOutcome::Stopped(_)));
    assert_eq!(
        deny_state
            .peek::<LastHttpPolicyV2Decision>()
            .and_then(|decision| decision.policy_action.as_deref()),
        Some("block")
    );
    assert_eq!(
        deny_confirmer.calls()[0].rule_id,
        "security.rules.http.ask_redirect"
    );
}

#[tokio::test]
async fn http_policy_v2_response_rewrite_mutates_header_value() {
    let pipeline = pipeline_for(
        r#"
[policy.http.rewrite_response_location]
on = "http.response"
if = 'response.status == "302"'
decision = "rewrite"
priority = 10
reason = "Route redirects through the allowed mirror"
rewrite_target = 'response.headers.location =~ "^https://github\.com/openai/(?P<repo>[^/?#]+)(?P<rest>.*)$"'
rewrite_value = "https://github.com/openclaw/${repo}${rest}"
"#,
    );
    let mut parts = response_parts();
    let mut state = HookState::default();

    let outcome = pipeline
        .dispatch(
            Event::RawResponseHead(&mut parts),
            &mut state,
            None,
            &conn(),
        )
        .await;

    assert!(matches!(outcome, DispatchOutcome::Completed));
    assert_eq!(
        parts
            .headers
            .get("location")
            .and_then(|value| value.to_str().ok()),
        Some("https://github.com/openclaw/capsem?ref=secret")
    );
    let decision = state
        .peek::<LastHttpPolicyV2Decision>()
        .expect("Policy V2 HTTP response rewrite decision should be stashed");
    assert_eq!(decision.policy_action.as_deref(), Some("rewrite"));
    assert_eq!(
        decision.policy_rule.as_deref(),
        Some("policy.http.rewrite_response_location")
    );
}

#[tokio::test]
async fn http_policy_v2_response_rewrite_rejects_unsupported_targets() {
    let pipeline = pipeline_for(
        r#"
[policy.http.rewrite_response_body]
on = "http.response"
if = 'response.status == "302"'
decision = "rewrite"
priority = 10
reason = "Body rewrites are not wired on the response-head path"
rewrite_target = 'response.body =~ "secret"'
rewrite_value = "[redacted]"
"#,
    );
    let mut parts = response_parts();
    let original_location = parts.headers.get("location").cloned();
    let mut state = HookState::default();

    let outcome = pipeline
        .dispatch(
            Event::RawResponseHead(&mut parts),
            &mut state,
            None,
            &conn(),
        )
        .await;

    assert!(matches!(outcome, DispatchOutcome::Stopped(_)));
    assert_eq!(
        parts.headers.get("location"),
        original_location.as_ref(),
        "failed response rewrites must not partially mutate the upstream response head"
    );
    let decision = state
        .peek::<LastHttpPolicyV2Decision>()
        .expect("Policy V2 HTTP response rewrite decision should be stashed");
    assert_eq!(decision.policy_action.as_deref(), Some("rewrite"));
    assert!(decision
        .policy_reason
        .as_deref()
        .is_some_and(|reason| reason.contains("unsupported HTTP response rewrite target")));
}
