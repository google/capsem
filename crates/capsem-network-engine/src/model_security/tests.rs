use std::collections::BTreeMap;

use capsem_security_engine::{
    AiApiFamily, AiAttributionScope, AiOriginKind, AiProvider, AiUsageEvidence, Enforceability,
    EvidenceStatus, ModelInteractionEvidence, ModelRequestEvidence, ParseStatus, RedactionState,
    SecurityEventCommon, SecurityEventSubject, SourceEngine,
};

use super::*;

#[test]
fn model_security_event_from_evidence_projects_canonical_subject() {
    let event = build_model_security_event_from_evidence(common(), evidence());

    assert_eq!(event.common.event_type, "model.request");
    match event.subject {
        SecurityEventSubject::Model(subject) => {
            assert_eq!(subject.provider, "openai");
            assert_eq!(subject.model, "gpt-5.5");
            assert_eq!(subject.estimated_input_tokens, Some(17));
            assert_eq!(subject.estimated_output_tokens, Some(23));
            assert_eq!(subject.estimated_cost_micros, Some(42));
            let evidence = subject.evidence.expect("evidence should be attached");
            assert_eq!(evidence.interaction_id, "model-int-1");
            assert_eq!(evidence.request.request_id, "request-1");
        }
        other => panic!("expected model subject, got {other:?}"),
    }
}

#[test]
fn model_security_event_supports_legacy_projection_without_evidence() {
    let event = build_model_security_event(
        common(),
        ModelSecurityEventInput {
            provider: "google".into(),
            model: "gemini-2.5-flash".into(),
            estimated_input_tokens: Some(3),
            estimated_output_tokens: Some(5),
            estimated_cost_micros: None,
            evidence: None,
        },
    );

    match event.subject {
        SecurityEventSubject::Model(subject) => {
            assert_eq!(subject.provider, "google");
            assert_eq!(subject.model, "gemini-2.5-flash");
            assert_eq!(subject.estimated_input_tokens, Some(3));
            assert_eq!(subject.estimated_output_tokens, Some(5));
            assert!(subject.evidence.is_none());
        }
        other => panic!("expected model subject, got {other:?}"),
    }
}

fn common() -> SecurityEventCommon {
    SecurityEventCommon {
        event_id: "evt-model-1".into(),
        parent_event_id: None,
        stream_id: None,
        activity_id: None,
        sequence_no: None,
        source_engine: SourceEngine::Network,
        attribution_scope: AiAttributionScope::Vm,
        origin_kind: AiOriginKind::GuestNetwork,
        accounting_owner: None,
        enforceability: Enforceability::ObserveOnly,
        trace_id: Some("trace-1".into()),
        span_id: None,
        timestamp_unix_ms: 123,
        vm_id: Some("vm-1".into()),
        session_id: Some("session-1".into()),
        profile_id: Some("profile-1".into()),
        profile_revision: None,
        profile_pack_ids: Vec::new(),
        enforcement_packs: Vec::new(),
        detection_packs: Vec::new(),
        user_id: Some("user-1".into()),
        process_id: None,
        parent_process_id: None,
        exec_id: None,
        turn_id: None,
        message_id: None,
        tool_call_id: None,
        mcp_call_id: None,
        event_type: "model.request".into(),
        redaction_state: RedactionState::Raw,
    }
}

fn evidence() -> ModelInteractionEvidence {
    ModelInteractionEvidence {
        interaction_id: "model-int-1".into(),
        trace_id: "trace-1".into(),
        attribution_scope: AiAttributionScope::Vm,
        source_engine: SourceEngine::Network,
        origin_kind: AiOriginKind::GuestNetwork,
        accounting_owner: None,
        profile_id: Some("profile-1".into()),
        vm_id: Some("vm-1".into()),
        session_id: Some("session-1".into()),
        user_id: Some("user-1".into()),
        provider: AiProvider::Openai,
        api_family: AiApiFamily::OpenaiResponses,
        model: "gpt-5.5".into(),
        request: ModelRequestEvidence {
            request_id: "request-1".into(),
            provider: AiProvider::Openai,
            api_family: AiApiFamily::OpenaiResponses,
            model: Some("gpt-5.5".into()),
            stream: true,
            system_prompt_preview: None,
            message_count: 1,
            tools_declared_count: 0,
            raw_shape_version: "openai.responses.v1".into(),
            unknown_fields_present: false,
        },
        response: None,
        tool_calls: Vec::new(),
        tool_results: Vec::new(),
        mcp_executions: Vec::new(),
        usage: AiUsageEvidence {
            input_tokens: Some(17),
            output_tokens: Some(23),
            estimated_cost_micros: Some(42),
            details: BTreeMap::new(),
        },
        parse_status: ParseStatus::Complete,
        evidence_status: EvidenceStatus::Complete,
    }
}
