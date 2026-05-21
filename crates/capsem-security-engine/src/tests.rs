use super::*;

#[test]
fn http_event_exposes_identity_and_quota_dimensions() {
    let event = SecurityEvent::http(
        SecurityEventCommon {
            event_id: "evt-1".into(),
            parent_event_id: Some("evt-parent".into()),
            stream_id: Some("stream-1".into()),
            activity_id: Some("activity-1".into()),
            sequence_no: Some(7),
            source_engine: SourceEngine::Network,
            enforceability: Enforceability::InlineBlockable,
            trace_id: Some("trace-1".into()),
            span_id: Some("span-1".into()),
            timestamp_unix_ms: 1_789,
            vm_id: Some("vm-1".into()),
            session_id: Some("session-1".into()),
            profile_id: Some("coding".into()),
            profile_revision: Some("rev-a".into()),
            profile_pack_ids: vec!["policy-pack".into(), "detection-pack".into()],
            user_id: Some("user-1".into()),
            process_id: Some("pid-1".into()),
            parent_process_id: Some("pid-0".into()),
            exec_id: Some("exec-1".into()),
            turn_id: Some("turn-1".into()),
            message_id: Some("msg-1".into()),
            tool_call_id: None,
            mcp_call_id: None,
            event_type: "http.request".into(),
            redaction_state: RedactionState::Raw,
        },
        HttpSecuritySubject {
            method: "POST".into(),
            host: "api.example.test".into(),
            path_class: "api-v1".into(),
            request_bytes: 512,
            response_bytes: None,
        },
    );

    let dims = event.quota_dimensions();
    assert_eq!(dims.profile_id.as_deref(), Some("coding"));
    assert_eq!(dims.profile_revision.as_deref(), Some("rev-a"));
    assert_eq!(dims.vm_id.as_deref(), Some("vm-1"));
    assert_eq!(dims.session_id.as_deref(), Some("session-1"));
    assert_eq!(dims.user_id.as_deref(), Some("user-1"));
    assert_eq!(dims.event_family, EventFamily::Http);
    assert_eq!(dims.event_type, "http.request");
    assert_eq!(
        dims.correlation_ids.parent_event_id.as_deref(),
        Some("evt-parent")
    );
    assert_eq!(dims.correlation_ids.stream_id.as_deref(), Some("stream-1"));
    assert_eq!(
        dims.correlation_ids.activity_id.as_deref(),
        Some("activity-1")
    );
    assert_eq!(dims.correlation_ids.sequence_no, Some(7));
    assert_eq!(dims.http_host.as_deref(), Some("api.example.test"));
    assert_eq!(dims.http_method.as_deref(), Some("POST"));
    assert_eq!(dims.http_path_class.as_deref(), Some("api-v1"));
    assert_eq!(dims.request_bytes, Some(512));
}

#[test]
fn resolved_event_roundtrips_throttle_and_rate_limit_step() {
    let event = SecurityEvent::model(
        SecurityEventCommon {
            event_id: "evt-model-1".into(),
            parent_event_id: None,
            stream_id: Some("model-stream-1".into()),
            activity_id: Some("model-activity-1".into()),
            sequence_no: Some(1),
            source_engine: SourceEngine::Network,
            enforceability: Enforceability::InlineBlockable,
            trace_id: None,
            span_id: None,
            timestamp_unix_ms: 1_790,
            vm_id: Some("vm-1".into()),
            session_id: Some("session-1".into()),
            profile_id: Some("coding".into()),
            profile_revision: Some("rev-a".into()),
            profile_pack_ids: Vec::new(),
            user_id: Some("user-1".into()),
            process_id: None,
            parent_process_id: None,
            exec_id: None,
            turn_id: Some("turn-1".into()),
            message_id: Some("msg-1".into()),
            tool_call_id: None,
            mcp_call_id: None,
            event_type: "model.request".into(),
            redaction_state: RedactionState::SummaryOnly,
        },
        ModelSecuritySubject {
            provider: "openai".into(),
            model: "gpt-5.5".into(),
            estimated_input_tokens: Some(1200),
            estimated_output_tokens: Some(400),
            estimated_cost_micros: Some(2500),
        },
    );

    let resolved = ResolvedSecurityEvent {
        event: event.clone(),
        steps: vec![ResolvedEventStep {
            kind: ResolvedEventStepKind::RateLimitCheck,
            status: StepStatus::Matched,
            rule_id: Some("quota-model-cost".into()),
            pack_id: None,
            message: Some("future quota provider would delay".into()),
        }],
        detection_findings: Vec::new(),
        final_action: SecurityAction::Throttle(ThrottlePlan {
            delay_ms: 250,
            quota_id: "model-cost-daily".into(),
            scope: "profile:coding".into(),
            reason_code: "budget_near_limit".into(),
            provider_source: Some("local".into()),
        }),
        emitter_results: vec![EmitterResult {
            sink: "session_db".into(),
            status: StepStatus::Applied,
            error: None,
        }],
    };

    let json = serde_json::to_string(&resolved).unwrap();
    assert!(json.contains("\"rate_limit_check\""));
    assert!(json.contains("\"throttle\""));

    let parsed: ResolvedSecurityEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, resolved);
    assert_eq!(
        parsed.event.quota_dimensions().provider.as_deref(),
        Some("openai")
    );
    assert_eq!(
        parsed.event.quota_dimensions().model.as_deref(),
        Some("gpt-5.5")
    );
    assert_eq!(
        parsed.event.quota_dimensions().estimated_cost_micros,
        Some(2500)
    );
}

#[test]
fn security_event_rejects_unknown_fields() {
    let err = serde_json::from_value::<SecurityEvent>(serde_json::json!({
        "common": {
            "event_id": "evt-unknown",
            "source_engine": "network",
            "enforceability": "inline_blockable",
            "timestamp_unix_ms": 1,
            "event_type": "dns.request",
            "redaction_state": "raw"
        },
        "subject": {
            "family": "dns",
            "qname": "example.test",
            "domain_class": "example",
            "extra": "must fail"
        }
    }))
    .unwrap_err();

    assert!(err.to_string().contains("unknown field"));
}
