use std::sync::Arc;

use crate::net::mitm_proxy::hooks::{ConnMeta, HookState};
use crate::net::mitm_proxy::pipeline::{DispatchOutcome, Pipeline};
use crate::net::mitm_proxy::protocol::Protocol;
use crate::net::policy_config::SettingsFile;

use super::*;

fn pipeline_for(toml_text: &str) -> Pipeline {
    let settings: SettingsFile = toml::from_str(toml_text).unwrap();
    let policy = Arc::new(tokio::sync::RwLock::new(Arc::new(settings.policy)));
    Pipeline::builder()
        .register(Arc::new(PolicyV2HttpHook::new(policy)))
        .build()
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
