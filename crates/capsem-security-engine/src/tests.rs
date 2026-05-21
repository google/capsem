use super::*;
use std::collections::{BTreeMap, BTreeSet};

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
            attribution_scope: AiAttributionScope::Vm,
            origin_kind: AiOriginKind::GuestNetwork,
            accounting_owner: Some("vm:vm-1".into()),
            enforceability: Enforceability::InlineBlockable,
            trace_id: Some("trace-1".into()),
            span_id: Some("span-1".into()),
            timestamp_unix_ms: 1_789,
            vm_id: Some("vm-1".into()),
            session_id: Some("session-1".into()),
            profile_id: Some("coding".into()),
            profile_revision: Some("rev-a".into()),
            profile_pack_ids: vec!["policy-pack".into(), "detection-pack".into()],
            enforcement_packs: Vec::new(),
            detection_packs: Vec::new(),
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
fn plugin_event_output_carries_ask_throttle_labels_findings_and_mutations() {
    let mut event = SecurityEvent::model(
        common("evt-plugin", "model.response", SourceEngine::Network),
        ModelSecuritySubject {
            provider: "openai".into(),
            model: "gpt-5.5".into(),
            estimated_input_tokens: None,
            estimated_output_tokens: Some(200),
            estimated_cost_micros: Some(1000),
            evidence: None,
        },
    );
    event.trace.labels.push("pii_access".into());
    event.context.history.push(TraceHistoryEntry {
        event_id: "evt-prev".into(),
        event_type: "file.read".into(),
        labels: vec!["pii_access".into()],
    });
    event.decision = Some(SecurityDecision {
        action: SecurityDecisionAction::Ask,
        rule: Some("plugin.pii-egress.ask".into()),
        reason: Some("open-world request after PII access".into()),
        terminal: false,
    });
    event.findings.push(DetectionFinding {
        finding_id: "finding-pii".into(),
        event_id: "evt-plugin".into(),
        rule_id: "pii-egress".into(),
        pack_id: "plugin-pack".into(),
        sigma_id: None,
        title: "PII egress risk".into(),
        severity: Severity::High,
        confidence: Confidence::High,
        tags: vec!["pii".into()],
    });
    event.mutations.push(EventMutation::ReplaceRegex {
        path: "subject.output_text".into(),
        pattern: "[0-9]{3}-[0-9]{2}-[0-9]{4}".into(),
        replacement: "[REDACTED]".into(),
        reason: Some("SSN-like value found".into()),
    });

    validate_plugin_output(&event).unwrap();

    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains("\"ask\""));
    assert!(json.contains("\"replace_regex\""));

    event.decision = Some(SecurityDecision {
        action: SecurityDecisionAction::Throttle,
        rule: Some("quota.future".into()),
        reason: Some("future quota check".into()),
        terminal: true,
    });
    validate_plugin_output(&event).unwrap();
}

#[test]
fn plugin_mutation_allowlist_rejects_illegal_targets() {
    let mut event = SecurityEvent::http(
        common("evt-http-mutate", "http.request", SourceEngine::Network),
        HttpSecuritySubject {
            method: "POST".into(),
            host: "api.example.test".into(),
            path_class: "api".into(),
            request_bytes: 10,
            response_bytes: None,
        },
    );
    event.mutations.push(EventMutation::StripHeader {
        path: "subject.headers.authorization".into(),
        reason: None,
    });
    validate_plugin_output(&event).unwrap();

    event.mutations.push(EventMutation::ReplaceRegex {
        path: "subject.output_text".into(),
        pattern: "secret".into(),
        replacement: "[REDACTED]".into(),
        reason: None,
    });

    let error = validate_plugin_output(&event).unwrap_err();
    assert!(error
        .to_string()
        .contains("mutation target is not allowed for http.request"));
}

#[test]
fn plugin_transform_preserves_core_event_and_records_hashes() {
    let mut input = SecurityEvent::http(
        common("evt-transform", "http.request", SourceEngine::Network),
        HttpSecuritySubject {
            method: "POST".into(),
            host: "api.example.test".into(),
            path_class: "api".into(),
            request_bytes: 10,
            response_bytes: None,
        },
    );
    input.labels.push("network".into());

    let mut output = input.clone();
    output.labels.push("pii_access".into());
    output.mutations.push(EventMutation::StripHeader {
        path: "subject.headers.authorization".into(),
        reason: Some("drop credential before egress".into()),
    });

    let plugin = PluginIdentity {
        id: "pii-egress".into(),
        version: "1.0.0".into(),
        hash: "blake3:plugin".into(),
    };
    let record = validate_plugin_transform(&plugin, &input, &output).unwrap();

    assert_eq!(record.plugin, plugin);
    assert_eq!(record.input_event_hash, canonical_event_hash(&input));
    assert_eq!(record.output_event_hash, canonical_event_hash(&output));
    assert_ne!(record.input_event_hash, record.output_event_hash);
    assert_eq!(
        validate_plugin_transform(&record.plugin, &input, &output).unwrap(),
        record
    );

    let resolved = ResolvedSecurityEvent {
        schema_version: RESOLVED_EVENT_SCHEMA_VERSION,
        event: output,
        steps: vec![ResolvedEventStep {
            kind: ResolvedEventStepKind::PluginCallback,
            status: StepStatus::Applied,
            rule_id: Some("pii-egress".into()),
            pack_id: Some("plugin-pack".into()),
            message: Some("plugin transform applied".into()),
        }],
        plugin_transforms: vec![record],
        detection_findings: Vec::new(),
        final_action: SecurityAction::Continue,
        emitter_results: Vec::new(),
    };

    assert_eq!(resolved.plugin_transforms[0].plugin.id, "pii-egress");
    assert_ne!(
        resolved.plugin_transforms[0].input_event_hash,
        resolved.plugin_transforms[0].output_event_hash
    );
}

#[test]
fn plugin_transform_rejects_hidden_subject_mutation() {
    let input = SecurityEvent::http(
        common("evt-hidden", "http.request", SourceEngine::Network),
        HttpSecuritySubject {
            method: "POST".into(),
            host: "api.example.test".into(),
            path_class: "api".into(),
            request_bytes: 10,
            response_bytes: None,
        },
    );
    let mut output = input.clone();
    output.subject = SecurityEventSubject::Http(HttpSecuritySubject {
        method: "POST".into(),
        host: "attacker.example.test".into(),
        path_class: "api".into(),
        request_bytes: 10,
        response_bytes: None,
    });

    let error = validate_plugin_transform(&plugin_identity(), &input, &output).unwrap_err();
    assert!(matches!(
        error,
        PluginValidationError::ImmutableFieldChanged { field: "subject" }
    ));
}

#[test]
fn plugin_transform_rejects_dropping_prior_findings_labels_or_mutations() {
    let mut input = SecurityEvent::http(
        common("evt-drop", "http.request", SourceEngine::Network),
        HttpSecuritySubject {
            method: "POST".into(),
            host: "api.example.test".into(),
            path_class: "api".into(),
            request_bytes: 10,
            response_bytes: None,
        },
    );
    input.labels.push("pii_access".into());
    input.findings.push(DetectionFinding {
        finding_id: "finding-existing".into(),
        event_id: "evt-drop".into(),
        rule_id: "rule-existing".into(),
        pack_id: "pack-existing".into(),
        sigma_id: None,
        title: "Existing finding".into(),
        severity: Severity::Medium,
        confidence: Confidence::High,
        tags: Vec::new(),
    });
    input.mutations.push(EventMutation::StripHeader {
        path: "subject.headers.authorization".into(),
        reason: None,
    });

    let mut output = input.clone();
    output.labels.clear();
    let error = validate_plugin_transform(&plugin_identity(), &input, &output).unwrap_err();
    assert!(matches!(
        error,
        PluginValidationError::PriorEventDataRemoved { field: "labels" }
    ));

    let mut output = input.clone();
    output.findings.clear();
    let error = validate_plugin_transform(&plugin_identity(), &input, &output).unwrap_err();
    assert!(matches!(
        error,
        PluginValidationError::PriorEventDataRemoved { field: "findings" }
    ));

    let mut output = input.clone();
    output.mutations.clear();
    let error = validate_plugin_transform(&plugin_identity(), &input, &output).unwrap_err();
    assert!(matches!(
        error,
        PluginValidationError::PriorEventDataRemoved { field: "mutations" }
    ));
}

#[test]
fn security_decision_projects_to_internal_transport_projection() {
    let mut event = SecurityEvent::http(
        common("evt-project", "http.request", SourceEngine::Network),
        HttpSecuritySubject {
            method: "GET".into(),
            host: "example.test".into(),
            path_class: "external".into(),
            request_bytes: 10,
            response_bytes: None,
        },
    );
    assert_eq!(
        project_transport_outcome(&event).unwrap(),
        TransportProjection::Continue
    );

    event.mutations.push(EventMutation::StripHeader {
        path: "subject.headers.authorization".into(),
        reason: None,
    });
    assert_eq!(
        project_transport_outcome(&event).unwrap(),
        TransportProjection::Rewrote
    );

    event.decision = Some(SecurityDecision {
        action: SecurityDecisionAction::Block,
        rule: Some("rule.block".into()),
        reason: Some("blocked".into()),
        terminal: true,
    });
    assert_eq!(
        project_transport_outcome(&event).unwrap(),
        TransportProjection::Stop
    );
}

#[test]
fn canonical_ai_evidence_fixture_covers_first_slice_providers_and_host_accounting() {
    let interactions: Vec<ModelInteractionEvidence> =
        serde_json::from_str(include_str!("../fixtures/ai-interaction-evidence-v1.json")).unwrap();

    let providers = interactions
        .iter()
        .map(|interaction| interaction.provider)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        providers,
        BTreeSet::from([
            AiProvider::Openai,
            AiProvider::Anthropic,
            AiProvider::GoogleGemini,
        ])
    );

    let openai = interactions
        .iter()
        .find(|interaction| interaction.interaction_id == "model-openai-tool-stream")
        .unwrap();
    assert_eq!(openai.api_family, AiApiFamily::OpenaiChatCompletions);
    assert_eq!(openai.tool_calls[0].origin, ToolOrigin::McpTool);
    assert_eq!(openai.mcp_executions[0].link_status, LinkStatus::Linked);
    assert!(openai.charges_vm_accounting());
    assert!(!openai.charges_host_accounting());

    let anthropic = interactions
        .iter()
        .find(|interaction| interaction.interaction_id == "model-anthropic-malformed-tool")
        .unwrap();
    assert_eq!(anthropic.api_family, AiApiFamily::AnthropicMessages);
    assert_eq!(
        anthropic.tool_calls[0].arguments_status,
        ArgumentsStatus::PartialJson
    );
    assert_eq!(anthropic.parse_status, ParseStatus::Partial);

    let gemini = interactions
        .iter()
        .find(|interaction| interaction.interaction_id == "model-gemini-function-response")
        .unwrap();
    assert_eq!(gemini.api_family, AiApiFamily::GoogleGeminiContent);
    assert_eq!(
        gemini.tool_results[0].result_status,
        ToolCallStatus::ReturnedToModel
    );
    assert!(gemini.tool_results[0].returned_to_model);

    let host_ai = interactions
        .iter()
        .find(|interaction| interaction.interaction_id == "host-ai-vm-name")
        .unwrap();
    assert_eq!(host_ai.source_engine, SourceEngine::HostAi);
    assert_eq!(host_ai.attribution_scope, AiAttributionScope::Host);
    assert_eq!(host_ai.origin_kind, AiOriginKind::HostService);
    assert_eq!(host_ai.vm_id.as_deref(), Some("vm-1"));
    assert!(host_ai.charges_host_accounting());
    assert!(!host_ai.charges_vm_accounting());
}

#[test]
fn model_security_subject_projects_canonical_evidence_to_quota_dimensions() {
    let evidence = model_interaction_evidence(
        "vm-model",
        AiAttributionScope::Vm,
        SourceEngine::Network,
        AiOriginKind::GuestNetwork,
        "vm:vm-1",
    );
    let mut common = common(
        "evt-evidence-model",
        "model.response",
        SourceEngine::Network,
    );
    common.attribution_scope = AiAttributionScope::Vm;
    common.origin_kind = AiOriginKind::GuestNetwork;
    common.accounting_owner = Some("vm:vm-1".into());
    let event = SecurityEvent::model(
        common,
        ModelSecuritySubject::from_interaction_evidence(evidence),
    );

    let dims = event.quota_dimensions();
    assert_eq!(dims.provider.as_deref(), Some("google_gemini"));
    assert_eq!(dims.model.as_deref(), Some("gemini-2.5-flash"));
    assert_eq!(dims.estimated_input_tokens, Some(40));
    assert_eq!(dims.estimated_output_tokens, Some(4));
    assert_eq!(dims.estimated_cost_micros, Some(12));
    assert_eq!(dims.attribution_scope, AiAttributionScope::Vm);
    assert_eq!(dims.accounting_owner.as_deref(), Some("vm:vm-1"));
    assert!(dims.charges_vm_accounting());
    assert!(!dims.charges_host_accounting());
}

#[test]
fn linked_model_and_mcp_evidence_project_to_policy_dimensions() {
    let mut evidence = model_interaction_evidence(
        "vm-model-linked",
        AiAttributionScope::Vm,
        SourceEngine::Network,
        AiOriginKind::GuestNetwork,
        "vm:vm-1",
    );
    evidence.tool_calls = vec![ModelToolCallEvidence {
        tool_call_id: "toolu-1".into(),
        index: 0,
        provider_call_id: Some("toolu-1".into()),
        raw_name: "filesystem__read_file".into(),
        normalized_name: "filesystem.read_file".into(),
        arguments_raw: Some(r#"{"path":"/tmp/a"}"#.into()),
        arguments_json: Some(r#"{"path":"/tmp/a"}"#.into()),
        arguments_status: ArgumentsStatus::ValidJson,
        origin: ToolOrigin::McpTool,
        linked_mcp_call_id: Some("mcp-1".into()),
        status: ToolCallStatus::Executed,
        parse_confidence: Confidence::High,
    }];
    evidence.tool_results = vec![ModelToolResultEvidence {
        tool_call_id: "toolu-1".into(),
        linked_mcp_call_id: Some("mcp-1".into()),
        content_kind: AiContentKind::Text,
        content_preview: Some("ok".into()),
        content_json: None,
        is_error: false,
        result_status: ToolCallStatus::ReturnedToModel,
        returned_to_model: true,
        parse_confidence: Confidence::High,
    }];
    evidence.mcp_executions = vec![McpToolExecutionEvidence {
        mcp_call_id: "mcp-1".into(),
        server_id: "filesystem".into(),
        tool_name: "read_file".into(),
        namespaced_tool_name: "filesystem.read_file".into(),
        transport: "mcp-framed".into(),
        request_arguments_raw: Some(r#"{"path":"/tmp/a"}"#.into()),
        request_arguments_json: Some(r#"{"path":"/tmp/a"}"#.into()),
        result_kind: AiContentKind::Text,
        result_preview: Some("ok".into()),
        result_json: None,
        is_error: false,
        latency_ms: 12,
        linked_model_interaction_id: Some("vm-model-linked".into()),
        linked_model_tool_call_id: Some("toolu-1".into()),
        link_status: LinkStatus::Linked,
    }];

    let model_event = SecurityEvent::model(
        common("evt-linked-model", "model.response", SourceEngine::Network),
        ModelSecuritySubject::from_interaction_evidence(evidence.clone()),
    );
    let model_dims = model_event.quota_dimensions();
    assert_eq!(
        model_dims.ai_api_family,
        Some(AiApiFamily::GoogleGeminiContent)
    );
    assert_eq!(
        model_dims.evidence_parse_status,
        Some(ParseStatus::Complete)
    );
    assert_eq!(model_dims.evidence_status, Some(EvidenceStatus::Complete));
    assert_eq!(model_dims.model_tool_call_count, Some(1));
    assert_eq!(model_dims.model_tool_result_count, Some(1));
    assert_eq!(model_dims.model_mcp_execution_count, Some(1));
    assert_eq!(model_dims.model_linked_mcp_tool_call_count, Some(1));

    let mcp_event = SecurityEvent::mcp(
        common("evt-linked-mcp", "mcp.request", SourceEngine::Network),
        McpSecuritySubject::from_execution_evidence(evidence.mcp_executions[0].clone()),
    );
    let mcp_dims = mcp_event.quota_dimensions();
    assert_eq!(mcp_dims.mcp_server.as_deref(), Some("filesystem"));
    assert_eq!(mcp_dims.mcp_tool.as_deref(), Some("read_file"));
    assert_eq!(mcp_dims.mcp_link_status, Some(LinkStatus::Linked));
    assert_eq!(
        mcp_dims.linked_model_interaction_id.as_deref(),
        Some("vm-model-linked")
    );
    assert_eq!(
        mcp_dims.linked_model_tool_call_id.as_deref(),
        Some("toolu-1")
    );
}

#[test]
fn host_ai_event_can_correlate_to_vm_without_charging_vm_accounting() {
    let evidence = model_interaction_evidence(
        "host-model",
        AiAttributionScope::Host,
        SourceEngine::HostAi,
        AiOriginKind::HostService,
        "host:service",
    );
    let event = SecurityEvent::model(
        common("evt-host-ai", "model.request", SourceEngine::HostAi),
        ModelSecuritySubject::from_interaction_evidence(evidence),
    );

    let dims = event.quota_dimensions();
    assert_eq!(dims.source_engine, SourceEngine::HostAi);
    assert_eq!(dims.origin_kind, AiOriginKind::HostService);
    assert_eq!(dims.attribution_scope, AiAttributionScope::Host);
    assert_eq!(dims.accounting_owner.as_deref(), Some("host:service"));
    assert_eq!(dims.vm_id.as_deref(), Some("vm-1"));
    assert_eq!(dims.session_id.as_deref(), Some("session-1"));
    assert!(dims.charges_host_accounting());
    assert!(!dims.charges_vm_accounting());
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
            attribution_scope: AiAttributionScope::Vm,
            origin_kind: AiOriginKind::GuestNetwork,
            accounting_owner: Some("vm:vm-1".into()),
            enforceability: Enforceability::InlineBlockable,
            trace_id: None,
            span_id: None,
            timestamp_unix_ms: 1_790,
            vm_id: Some("vm-1".into()),
            session_id: Some("session-1".into()),
            profile_id: Some("coding".into()),
            profile_revision: Some("rev-a".into()),
            profile_pack_ids: Vec::new(),
            enforcement_packs: Vec::new(),
            detection_packs: Vec::new(),
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
            evidence: None,
        },
    );

    let resolved = ResolvedSecurityEvent {
        schema_version: RESOLVED_EVENT_SCHEMA_VERSION,
        event: event.clone(),
        steps: vec![ResolvedEventStep {
            kind: ResolvedEventStepKind::RateLimitCheck,
            status: StepStatus::Matched,
            rule_id: Some("quota-model-cost".into()),
            pack_id: None,
            message: Some("future quota provider would delay".into()),
        }],
        plugin_transforms: Vec::new(),
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
fn security_action_roundtrips_ask() {
    let action = SecurityAction::Ask(AskPlan {
        prompt_id: "ask-1".into(),
        reason_code: "plugin_requested_confirmation".into(),
        default_action: Box::new(SecurityAction::Block(BlockResponse {
            reason_code: "ask_timeout".into(),
            rule_id: Some("plugin.pii-egress.ask".into()),
        })),
    });

    let json = serde_json::to_string(&action).unwrap();
    assert!(json.contains("\"ask\""));

    let parsed: SecurityAction = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, action);
}

fn common(event_id: &str, event_type: &str, source_engine: SourceEngine) -> SecurityEventCommon {
    SecurityEventCommon {
        event_id: event_id.into(),
        parent_event_id: None,
        stream_id: None,
        activity_id: None,
        sequence_no: None,
        source_engine,
        attribution_scope: if source_engine == SourceEngine::HostAi {
            AiAttributionScope::Host
        } else {
            AiAttributionScope::Vm
        },
        origin_kind: if source_engine == SourceEngine::HostAi {
            AiOriginKind::HostService
        } else {
            AiOriginKind::GuestNetwork
        },
        accounting_owner: Some(if source_engine == SourceEngine::HostAi {
            "host:service".into()
        } else {
            "vm:vm-1".into()
        }),
        enforceability: Enforceability::InlineBlockable,
        trace_id: Some("trace-plugin".into()),
        span_id: None,
        timestamp_unix_ms: 1_789,
        vm_id: Some("vm-1".into()),
        session_id: Some("session-1".into()),
        profile_id: Some("coding".into()),
        profile_revision: Some("rev-a".into()),
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
        event_type: event_type.into(),
        redaction_state: RedactionState::Raw,
    }
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

#[test]
fn security_event_fixture_covers_every_family_and_pack_identity() {
    let events: Vec<SecurityEvent> =
        serde_json::from_str(include_str!("../fixtures/security-events-v1.json")).unwrap();

    let families = events
        .iter()
        .map(SecurityEvent::event_family)
        .collect::<BTreeSet<_>>();

    assert_eq!(
        families,
        BTreeSet::from([
            EventFamily::Dns,
            EventFamily::Http,
            EventFamily::Mcp,
            EventFamily::Model,
            EventFamily::File,
            EventFamily::Process,
            EventFamily::Credential,
            EventFamily::Vm,
            EventFamily::Profile,
            EventFamily::Conversation,
            EventFamily::Snapshot,
        ])
    );

    assert!(events
        .iter()
        .all(|event| event.schema_version == SECURITY_EVENT_SCHEMA_VERSION));

    let http = events
        .iter()
        .find(|event| event.common.event_id == "evt-http")
        .unwrap();
    assert_eq!(http.common.enforcement_packs[0].id, "corp-enforcement");
    assert_eq!(http.common.detection_packs[0].id, "corp-detection");
    assert_eq!(http.trace.labels, vec!["pii_access"]);
    assert_eq!(http.labels, vec!["metadata_access"]);
    assert_eq!(
        http.decision.as_ref().unwrap().action,
        SecurityDecisionAction::Ask
    );
    assert!(matches!(
        http.mutations[0],
        EventMutation::StripHeader { .. }
    ));
}

#[test]
fn resolved_event_fixture_pins_schema_version_and_findings() {
    let resolved: ResolvedSecurityEvent =
        serde_json::from_str(include_str!("../fixtures/resolved-event-v1.json")).unwrap();

    assert_eq!(resolved.schema_version, RESOLVED_EVENT_SCHEMA_VERSION);
    assert_eq!(resolved.event.schema_version, SECURITY_EVENT_SCHEMA_VERSION);
    assert_eq!(resolved.detection_findings[0].finding_id, "finding-1");
    assert_eq!(resolved.detection_findings[0].event_id, "evt-http");
    assert_eq!(resolved.event.labels, vec!["metadata_access"]);
    assert_eq!(
        resolved.event.decision.as_ref().unwrap().action,
        SecurityDecisionAction::Allow
    );
    assert!(matches!(resolved.final_action, SecurityAction::Continue));
}

#[test]
fn resolved_event_emitter_records_sink_delivery_and_shared_ids() {
    let mut emitter = ResolvedEventEmitter::default();
    emitter.add_sink(Box::new(RecordingSink::new(
        "session_db",
        SinkRequirement::Required,
    )));
    emitter.add_sink(Box::new(RecordingSink::new(
        "telemetry",
        SinkRequirement::BestEffort,
    )));

    let outcome = emitter.emit(resolved_event_with_finding("evt-emit", "finding-emit"));

    assert!(!outcome.required_sink_failed);
    assert_eq!(outcome.resolved_event.emitter_results.len(), 2);
    assert!(outcome
        .resolved_event
        .emitter_results
        .iter()
        .all(|result| result.status == StepStatus::Applied));
    assert_eq!(
        emitter.deliveries(),
        &[
            SinkDelivery {
                sink: "session_db".into(),
                event_id: "evt-emit".into(),
                finding_ids: vec!["finding-emit".into()],
            },
            SinkDelivery {
                sink: "telemetry".into(),
                event_id: "evt-emit".into(),
                finding_ids: vec!["finding-emit".into()],
            },
        ]
    );
}

#[test]
fn resolved_event_emitter_marks_required_sink_failure() {
    let mut emitter = ResolvedEventEmitter::default();
    emitter.add_sink(Box::new(FailingSink::new(
        "session_db",
        SinkRequirement::Required,
    )));
    emitter.add_sink(Box::new(RecordingSink::new(
        "telemetry",
        SinkRequirement::BestEffort,
    )));

    let outcome = emitter.emit(resolved_event_with_finding("evt-fail", "finding-fail"));

    assert!(outcome.required_sink_failed);
    assert_eq!(outcome.resolved_event.emitter_results.len(), 2);
    assert_eq!(outcome.resolved_event.emitter_results[0].sink, "session_db");
    assert_eq!(
        outcome.resolved_event.emitter_results[0].status,
        StepStatus::Error
    );
    assert_eq!(outcome.resolved_event.emitter_results[1].sink, "telemetry");
    assert_eq!(
        outcome.resolved_event.emitter_results[1].status,
        StepStatus::Applied
    );
}

#[test]
fn backtest_rows_dedupe_by_evidence_signature_and_limit_to_default() {
    let rows = (0..130)
        .map(|index| BacktestMatchRow {
            event_ref: BacktestEventRef {
                corpus: "session".into(),
                session_id: Some("session-1".into()),
                event_id: format!("evt-{index}"),
                sequence_no: Some(index),
                timestamp_unix_ms: 1_789 + index,
            },
            rule_id: "rule-1".into(),
            pack_id: "pack-1".into(),
            evidence_signature: format!("signature-{}", index % 110),
            matched_fields: Vec::new(),
            outcome: BacktestOutcome::Matched,
        })
        .collect();

    let result = dedupe_backtest_matches(rows, DEFAULT_BACKTEST_MATCH_LIMIT);

    assert_eq!(result.total_matches, 130);
    assert_eq!(result.unique_evidence_matches, 110);
    assert_eq!(result.rows.len(), DEFAULT_BACKTEST_MATCH_LIMIT);
    assert_eq!(result.rows[0].event_ref.event_id, "evt-0");
    assert_eq!(result.rows[99].event_ref.event_id, "evt-99");
    assert!(result.truncated);
}

#[test]
fn backtest_rows_keep_mismatches_and_full_event_refs() {
    let rows = vec![
        BacktestMatchRow {
            event_ref: BacktestEventRef {
                corpus: "fixture".into(),
                session_id: None,
                event_id: "evt-a".into(),
                sequence_no: Some(4),
                timestamp_unix_ms: 44,
            },
            rule_id: "rule-a".into(),
            pack_id: "pack-a".into(),
            evidence_signature: "same".into(),
            matched_fields: vec![MatchedField {
                path: "subject.request.host".into(),
                value: serde_json::json!("metadata"),
            }],
            outcome: BacktestOutcome::Mismatch {
                expected: "no_match".into(),
                actual: "matched".into(),
            },
        },
        BacktestMatchRow {
            event_ref: BacktestEventRef {
                corpus: "fixture".into(),
                session_id: None,
                event_id: "evt-b".into(),
                sequence_no: Some(5),
                timestamp_unix_ms: 45,
            },
            rule_id: "rule-a".into(),
            pack_id: "pack-a".into(),
            evidence_signature: "same".into(),
            matched_fields: Vec::new(),
            outcome: BacktestOutcome::Matched,
        },
    ];

    let result = dedupe_backtest_matches(rows, 100);

    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0].event_ref.corpus, "fixture");
    assert_eq!(result.rows[0].event_ref.sequence_no, Some(4));
    assert!(matches!(
        result.rows[0].outcome,
        BacktestOutcome::Mismatch { .. }
    ));
}

#[test]
fn runtime_rule_registry_keeps_previous_plan_when_update_fails() {
    let mut registry = RuntimeRuleRegistry::default();
    registry
        .add_or_update(
            RuntimeRuleRecord {
                metadata: rule_metadata("deny-metadata"),
                source: "host == '169.254.169.254'".into(),
                enabled: true,
            },
            compile_rule_source,
        )
        .unwrap();

    let err = registry
        .add_or_update(
            RuntimeRuleRecord {
                metadata: rule_metadata("deny-metadata"),
                source: "invalid cel".into(),
                enabled: true,
            },
            compile_rule_source,
        )
        .unwrap_err();

    assert!(err.to_string().contains("invalid"));
    let listed = registry.list();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].metadata.id, "deny-metadata");
    assert_eq!(listed[0].source, "host == '169.254.169.254'");
    assert!(matches!(listed[0].compile_status, CompileStatus::Compiled));
    assert_eq!(listed[0].generation, 1);
}

#[test]
fn runtime_rule_registry_tracks_match_stats_and_delete() {
    let mut registry = RuntimeRuleRegistry::default();
    registry
        .add_or_update(
            RuntimeRuleRecord {
                metadata: rule_metadata("detect-metadata"),
                source: "host == '169.254.169.254'".into(),
                enabled: true,
            },
            compile_rule_source,
        )
        .unwrap();

    registry
        .record_match("detect-metadata", "evt-1", 1_789)
        .unwrap();
    registry
        .record_match("detect-metadata", "evt-2", 1_790)
        .unwrap();

    let stats = registry.stats("detect-metadata").unwrap();
    assert_eq!(stats.match_count, 2);
    assert_eq!(stats.last_matched_event.as_deref(), Some("evt-2"));
    assert_eq!(stats.last_matched_unix_ms, Some(1_790));

    let removed = registry.delete("detect-metadata").unwrap();
    assert_eq!(removed.metadata.id, "detect-metadata");
    assert!(registry.list().is_empty());
}

fn rule_metadata(id: &str) -> RuntimeRuleMetadata {
    RuntimeRuleMetadata {
        id: id.into(),
        pack_id: Some("pack-1".into()),
        scope: RuleScope::Runtime,
        origin: RuleOrigin::Runtime,
    }
}

fn compile_rule_source(source: &str) -> Result<String, RuleRegistryError> {
    if source.contains("invalid") {
        Err(RuleRegistryError::CompileFailed("invalid rule".into()))
    } else {
        Ok(format!("compiled:{source}"))
    }
}

fn plugin_identity() -> PluginIdentity {
    PluginIdentity {
        id: "pii-egress".into(),
        version: "1.0.0".into(),
        hash: "blake3:plugin".into(),
    }
}

fn model_interaction_evidence(
    interaction_id: &str,
    attribution_scope: AiAttributionScope,
    source_engine: SourceEngine,
    origin_kind: AiOriginKind,
    accounting_owner: &str,
) -> ModelInteractionEvidence {
    ModelInteractionEvidence {
        interaction_id: interaction_id.into(),
        trace_id: "trace-ai".into(),
        attribution_scope,
        source_engine,
        origin_kind,
        accounting_owner: Some(accounting_owner.into()),
        profile_id: Some("coding".into()),
        vm_id: Some("vm-1".into()),
        session_id: Some("session-1".into()),
        user_id: Some("user-1".into()),
        provider: AiProvider::GoogleGemini,
        api_family: AiApiFamily::GoogleGeminiContent,
        model: "gemini-2.5-flash".into(),
        request: ModelRequestEvidence {
            request_id: format!("req-{interaction_id}"),
            provider: AiProvider::GoogleGemini,
            api_family: AiApiFamily::GoogleGeminiContent,
            model: Some("gemini-2.5-flash".into()),
            stream: false,
            system_prompt_preview: Some("summarize session".into()),
            message_count: 1,
            tools_declared_count: 0,
            raw_shape_version: "host_ai.prompt.v1".into(),
            unknown_fields_present: false,
        },
        response: Some(ModelResponseEvidence {
            response_id: format!("resp-{interaction_id}"),
            provider_response_id: None,
            stop_reason: Some("stop".into()),
            text_preview: Some("Winter Build".into()),
            thinking_preview: None,
            content_blocks: vec![AiContentBlock::Text {
                text_preview: "Winter Build".into(),
            }],
            usage: AiUsageEvidence {
                input_tokens: Some(40),
                output_tokens: Some(4),
                estimated_cost_micros: Some(12),
                details: BTreeMap::new(),
            },
            raw_shape_version: "host_ai.prompt.v1".into(),
        }),
        tool_calls: Vec::new(),
        tool_results: Vec::new(),
        mcp_executions: Vec::new(),
        usage: AiUsageEvidence {
            input_tokens: Some(40),
            output_tokens: Some(4),
            estimated_cost_micros: Some(12),
            details: BTreeMap::new(),
        },
        parse_status: ParseStatus::Complete,
        evidence_status: EvidenceStatus::Complete,
    }
}

fn resolved_event_with_finding(event_id: &str, finding_id: &str) -> ResolvedSecurityEvent {
    let event = SecurityEvent::http(
        SecurityEventCommon {
            event_id: event_id.into(),
            parent_event_id: None,
            stream_id: None,
            activity_id: None,
            sequence_no: None,
            source_engine: SourceEngine::Network,
            attribution_scope: AiAttributionScope::Vm,
            origin_kind: AiOriginKind::GuestNetwork,
            accounting_owner: Some("vm:vm-1".into()),
            enforceability: Enforceability::InlineBlockable,
            trace_id: None,
            span_id: None,
            timestamp_unix_ms: 1_789,
            vm_id: Some("vm-1".into()),
            session_id: Some("session-1".into()),
            profile_id: Some("coding".into()),
            profile_revision: Some("rev-a".into()),
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
            event_type: "http.request".into(),
            redaction_state: RedactionState::Raw,
        },
        HttpSecuritySubject {
            method: "GET".into(),
            host: "example.test".into(),
            path_class: "external".into(),
            request_bytes: 64,
            response_bytes: None,
        },
    );

    ResolvedSecurityEvent {
        schema_version: RESOLVED_EVENT_SCHEMA_VERSION,
        event,
        steps: Vec::new(),
        plugin_transforms: Vec::new(),
        detection_findings: vec![DetectionFinding {
            finding_id: finding_id.into(),
            event_id: event_id.into(),
            rule_id: "rule-1".into(),
            pack_id: "pack-1".into(),
            sigma_id: None,
            title: "finding".into(),
            severity: Severity::Medium,
            confidence: Confidence::High,
            tags: Vec::new(),
        }],
        final_action: SecurityAction::Continue,
        emitter_results: Vec::new(),
    }
}

struct RecordingSink {
    name: String,
    requirement: SinkRequirement,
}

impl RecordingSink {
    fn new(name: &str, requirement: SinkRequirement) -> Self {
        Self {
            name: name.into(),
            requirement,
        }
    }
}

impl ResolvedEventSink for RecordingSink {
    fn name(&self) -> &str {
        &self.name
    }

    fn requirement(&self) -> SinkRequirement {
        self.requirement
    }

    fn emit(&mut self, event: &ResolvedSecurityEvent) -> Result<(), EmitterError> {
        assert_eq!(event.schema_version, RESOLVED_EVENT_SCHEMA_VERSION);
        Ok(())
    }
}

struct FailingSink {
    name: String,
    requirement: SinkRequirement,
}

impl FailingSink {
    fn new(name: &str, requirement: SinkRequirement) -> Self {
        Self {
            name: name.into(),
            requirement,
        }
    }
}

impl ResolvedEventSink for FailingSink {
    fn name(&self) -> &str {
        &self.name
    }

    fn requirement(&self) -> SinkRequirement {
        self.requirement
    }

    fn emit(&mut self, _event: &ResolvedSecurityEvent) -> Result<(), EmitterError> {
        Err(EmitterError::new("sink unavailable"))
    }
}
