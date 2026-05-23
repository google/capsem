use super::*;

fn http_input() -> HttpSecurityEventInput {
    HttpSecurityEventInput {
        event_id_seed: "test-request-seed".into(),
        domain: "api.anthropic.com".into(),
        method: "POST".into(),
        path: "/v1/messages".into(),
        query: None,
        status_code: Some(200),
        request_headers: Some("Host: api.anthropic.com\nAuthorization: bearer token".into()),
        response_headers: Some("content-type: text/event-stream".into()),
        request_bytes: 37,
        request_body_preview: Some("{\"model\":\"claude-test\",\"messages\":[]}".into()),
        response_bytes: Some(4567),
        response_body_preview: Some("chunk-preview".into()),
        port: 443,
        conn_type: "https-mitm".into(),
        identity: HttpIdentityContext::default(),
        decision: Decision::Allowed,
        matched_rule: Some("default-dev-allow".into()),
        policy_rule: None,
        policy_reason: None,
    }
}

#[test]
fn http_event_id_seed_prevents_same_millisecond_collisions() {
    let timestamp_unix_ms = 1779544024000;
    let mut first = http_input();
    let mut second = http_input();
    first.event_id_seed = "same-ms-request-1".into();
    second.event_id_seed = "same-ms-request-2".into();

    let first_event =
        build_http_security_event(&first, timestamp_unix_ms, Some("trace-winterfell".into()));
    let second_event =
        build_http_security_event(&second, timestamp_unix_ms, Some("trace-winterfell".into()));

    assert_ne!(first_event.common.event_id, second_event.common.event_id);
}

#[test]
fn build_http_resolved_security_event_carries_http_subject_and_allow_action() {
    let resolved =
        build_http_resolved_security_event(&http_input(), 1779544024000, Some("trace-a".into()));

    assert_eq!(resolved.event.common.event_type, "http.request");
    assert_eq!(resolved.event.common.source_engine, SourceEngine::Network);
    assert_eq!(
        resolved.event.common.attribution_scope,
        AiAttributionScope::Vm
    );
    assert!(matches!(resolved.final_action, SecurityAction::Continue));
    let capsem_security_engine::SecurityEventSubject::Http(subject) = &resolved.event.subject
    else {
        panic!("expected http subject");
    };
    assert_eq!(subject.method, "POST");
    assert_eq!(subject.host, "api.anthropic.com");
    assert_eq!(subject.port, Some(443));
    assert_eq!(subject.path.as_deref(), Some("/v1/messages"));
    assert_eq!(
        subject.url.as_deref(),
        Some("https://api.anthropic.com/v1/messages")
    );
    assert_eq!(subject.path_class, "v1");
    assert_eq!(subject.request_bytes, 37);
    assert_eq!(subject.response_status, Some(200));
    assert_eq!(subject.response_bytes, Some(4567));
    assert_eq!(
        subject
            .request_headers
            .get("authorization")
            .and_then(|values| values.first())
            .map(String::as_str),
        Some("bearer token")
    );
    assert_eq!(
        subject
            .response_body
            .as_ref()
            .and_then(|body| body.text.as_deref()),
        Some("chunk-preview")
    );
}

#[test]
fn build_http_resolved_security_event_carries_identity() {
    let mut input = http_input();
    input.identity = HttpIdentityContext {
        vm_id: Some("vm-winterfell".into()),
        session_id: Some("session-winterfell".into()),
        profile_id: Some("coding".into()),
        profile_revision: Some("2026.0522.1".into()),
        user_id: Some("arya".into()),
    };

    let resolved = build_http_resolved_security_event(&input, 1779544024000, None);

    assert_eq!(
        resolved.event.common.vm_id.as_deref(),
        Some("vm-winterfell")
    );
    assert_eq!(
        resolved.event.common.session_id.as_deref(),
        Some("session-winterfell")
    );
    assert_eq!(resolved.event.common.profile_id.as_deref(), Some("coding"));
    assert_eq!(
        resolved.event.common.profile_revision.as_deref(),
        Some("2026.0522.1")
    );
    assert_eq!(resolved.event.common.user_id.as_deref(), Some("arya"));
}

#[test]
fn build_http_resolved_security_event_maps_denied_decision_to_block() {
    let mut input = http_input();
    input.decision = Decision::Denied;
    input.status_code = Some(403);
    input.matched_rule = Some("runtime.block_metadata".into());
    input.policy_rule = Some("policy.http.block_metadata".into());
    input.policy_reason = Some("metadata access".into());

    let resolved = build_http_resolved_security_event(&input, 1779544024000, None);

    assert!(matches!(resolved.final_action, SecurityAction::Block(_)));
    assert_eq!(
        resolved
            .event
            .decision
            .as_ref()
            .and_then(|d| d.rule.as_deref()),
        Some("policy.http.block_metadata")
    );
    assert_eq!(resolved.steps.len(), 1);
    assert_eq!(
        resolved.steps[0].kind,
        ResolvedEventStepKind::EnforcementMatch
    );
    assert_eq!(resolved.steps[0].status, StepStatus::Matched);
}
