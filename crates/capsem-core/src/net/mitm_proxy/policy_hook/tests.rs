use super::super::events::Event;
use super::super::hooks::{ConnMeta, HookOutcome, HookState, StopAction};
use super::super::pipeline::{DispatchOutcome, Pipeline};
use super::*;
use crate::net::policy::PolicyRule;
use std::sync::{Arc, RwLock};

fn allow_rule(pattern: &str) -> PolicyRule {
    use crate::net::policy::DomainMatcher;
    PolicyRule {
        matcher: DomainMatcher::parse(pattern),
        allow_read: true,
        allow_write: true,
    }
}

fn make_policy(allowed_domains: Vec<&str>, default_allow: bool) -> LivePolicy {
    let rules: Vec<PolicyRule> = allowed_domains.into_iter().map(allow_rule).collect();
    let policy = NetworkPolicy::new(rules, default_allow, default_allow);
    Arc::new(RwLock::new(Arc::new(policy)))
}

fn make_request_head(method: &str) -> http::request::Parts {
    http::Request::builder()
        .method(method)
        .uri("/v1/messages")
        .body(())
        .unwrap()
        .into_parts()
        .0
}

async fn dispatch(
    pipeline: &Pipeline,
    parts: &mut http::request::Parts,
    domain: &str,
) -> DispatchOutcome {
    let mut state = HookState::default();
    let conn = ConnMeta {
        domain: domain.to_string(),
        port: 443,
        process_name: None,
        ..Default::default()
    };
    pipeline
        .dispatch(Event::RawRequestHead(parts), &mut state, None, &conn)
        .await
}

#[tokio::test]
async fn allowed_domain_continues() {
    let pipeline = Pipeline::builder()
        .register(Arc::new(PolicyHook::new(make_policy(
            vec!["api.anthropic.com"],
            false,
        ))))
        .build();
    let mut parts = make_request_head("GET");
    let out = dispatch(&pipeline, &mut parts, "api.anthropic.com").await;
    assert!(matches!(out, DispatchOutcome::Completed));
}

#[tokio::test]
async fn denied_domain_returns_stop_reject_403() {
    let pipeline = Pipeline::builder()
        .register(Arc::new(PolicyHook::new(make_policy(
            vec!["api.anthropic.com"],
            false,
        ))))
        .build();
    let mut parts = make_request_head("GET");
    let out = dispatch(&pipeline, &mut parts, "evil.example.com").await;
    let resp = match out {
        DispatchOutcome::Stopped(StopAction::Reject(r)) => r,
        other => panic!("expected Reject, got {:?}", std::mem::discriminant(&other)),
    };
    assert_eq!(resp.status(), http::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn default_allow_passes_unknown_domain() {
    let pipeline = Pipeline::builder()
        .register(Arc::new(PolicyHook::new(make_policy(vec![], true))))
        .build();
    let mut parts = make_request_head("GET");
    let out = dispatch(&pipeline, &mut parts, "anything.example").await;
    assert!(matches!(out, DispatchOutcome::Completed));
}

#[tokio::test]
async fn evaluate_decision_branches() {
    // Verify the helper used by both the hook and (in slice 2c) the
    // inline call site renders the right HookOutcome for allow vs
    // deny PolicyDecisions.
    let allow_dec = PolicyDecision {
        allowed: true,
        matched_rule: "test".into(),
        reason: "ok".into(),
    };
    let allow = evaluate_decision(&allow_dec, "x.com", "GET");
    assert!(matches!(allow, HookOutcome::Continue));

    let deny_dec = PolicyDecision {
        allowed: false,
        matched_rule: "test".into(),
        reason: "blocked".into(),
    };
    let deny = evaluate_decision(&deny_dec, "x.com", "POST");
    let resp = match deny {
        HookOutcome::Stop(StopAction::Reject(r)) => r,
        _ => panic!("expected Reject"),
    };
    assert_eq!(resp.status(), http::StatusCode::FORBIDDEN);
}
