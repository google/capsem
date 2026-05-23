use std::time::SystemTime;

use capsem_security_engine::{
    AiAttributionScope, AiOriginKind, BlockResponse, Enforceability, McpSecuritySubject,
    RedactionState, ResolvedEventStep, ResolvedEventStepKind, ResolvedSecurityEvent,
    SecurityAction, SecurityDecision, SecurityDecisionAction, SecurityError, SecurityEvent,
    SecurityEventCommon, SecurityResult, SourceEngine, StepStatus, RESOLVED_EVENT_SCHEMA_VERSION,
};

const CAPSEM_VM_ID_ENV: &str = "CAPSEM_VM_ID";
const CAPSEM_SESSION_ID_ENV: &str = "CAPSEM_SESSION_ID";
const CAPSEM_PROFILE_ID_ENV: &str = "CAPSEM_PROFILE_ID";
const CAPSEM_PROFILE_REVISION_ENV: &str = "CAPSEM_PROFILE_REVISION";
const CAPSEM_USER_ID_ENV: &str = "CAPSEM_USER_ID";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpPolicyFields {
    pub policy_action: Option<String>,
    pub policy_rule: Option<String>,
    pub policy_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpSecurityEventInput {
    pub server_name: String,
    pub tool_name: String,
    pub request_id: Option<String>,
    pub policy_fields: McpPolicyFields,
    pub decision: Option<String>,
    pub response_error_message: Option<String>,
}

pub fn build_mcp_security_event(
    input: &McpSecurityEventInput,
    trace_id: Option<String>,
    timestamp: SystemTime,
) -> SecurityEvent {
    let timestamp_duration = timestamp
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let timestamp_unix_ms = timestamp_duration.as_millis() as u64;
    let timestamp_unix_nanos = timestamp_duration.as_nanos();
    let event_id = mcp_security_event_id(
        trace_id.as_deref(),
        &input.server_name,
        &input.tool_name,
        input.request_id.as_deref(),
        timestamp_unix_nanos,
    );

    SecurityEvent::mcp(
        SecurityEventCommon {
            event_id,
            parent_event_id: None,
            stream_id: None,
            activity_id: None,
            sequence_no: None,
            source_engine: SourceEngine::Network,
            attribution_scope: AiAttributionScope::Vm,
            origin_kind: AiOriginKind::GuestNetwork,
            accounting_owner: None,
            enforceability: Enforceability::InlineBlockable,
            trace_id,
            span_id: None,
            timestamp_unix_ms,
            vm_id: non_empty_env(CAPSEM_VM_ID_ENV),
            session_id: non_empty_env(CAPSEM_SESSION_ID_ENV),
            profile_id: non_empty_env(CAPSEM_PROFILE_ID_ENV),
            profile_revision: non_empty_env(CAPSEM_PROFILE_REVISION_ENV),
            profile_pack_ids: Vec::new(),
            enforcement_packs: Vec::new(),
            detection_packs: Vec::new(),
            user_id: non_empty_env(CAPSEM_USER_ID_ENV),
            process_id: None,
            parent_process_id: None,
            exec_id: None,
            turn_id: None,
            message_id: None,
            tool_call_id: input.request_id.clone(),
            mcp_call_id: input.request_id.clone(),
            event_type: "mcp.request".into(),
            redaction_state: RedactionState::Raw,
        },
        McpSecuritySubject {
            server_id: input.server_name.clone(),
            tool_name: input.tool_name.clone(),
            evidence: None,
        },
    )
}

pub fn build_mcp_resolved_security_event(
    input: &McpSecurityEventInput,
    trace_id: Option<String>,
    timestamp: SystemTime,
) -> ResolvedSecurityEvent {
    let mut event = build_mcp_security_event(input, trace_id, timestamp);
    let mut steps = Vec::new();
    if let Some(action) = input
        .policy_fields
        .policy_action
        .as_deref()
        .and_then(mcp_security_decision_action)
    {
        event.decision = Some(SecurityDecision {
            action,
            rule: input.policy_fields.policy_rule.clone(),
            pack_id: None,
            reason: input.policy_fields.policy_reason.clone(),
            terminal: matches!(
                action,
                SecurityDecisionAction::Ask
                    | SecurityDecisionAction::Block
                    | SecurityDecisionAction::Rewrite
                    | SecurityDecisionAction::Throttle
            ),
        });
        steps.push(ResolvedEventStep {
            kind: ResolvedEventStepKind::EnforcementMatch,
            status: StepStatus::Matched,
            rule_id: input.policy_fields.policy_rule.clone(),
            pack_id: None,
            message: input.policy_fields.policy_reason.clone(),
        });
    }

    let final_action = match input.decision.as_deref() {
        Some("denied") => SecurityAction::Block(BlockResponse {
            reason_code: input
                .policy_fields
                .policy_reason
                .clone()
                .unwrap_or_else(|| "mcp_call_denied".into()),
            rule_id: input.policy_fields.policy_rule.clone(),
        }),
        Some("error") => SecurityAction::Error(SecurityError {
            code: "mcp_error".into(),
            message: input
                .response_error_message
                .clone()
                .unwrap_or_else(|| "MCP call failed".into()),
        }),
        _ => SecurityAction::Continue,
    };

    ResolvedSecurityEvent {
        schema_version: RESOLVED_EVENT_SCHEMA_VERSION,
        event,
        steps,
        plugin_transforms: Vec::new(),
        detection_findings: Vec::new(),
        final_action,
        emitter_results: Vec::new(),
    }
}

pub fn mcp_security_result_allows_dispatch(result: &SecurityResult) -> bool {
    matches!(
        result.action,
        SecurityAction::Continue | SecurityAction::ObserveOnly
    )
}

fn non_empty_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn mcp_security_decision_action(action: &str) -> Option<SecurityDecisionAction> {
    match action {
        "allow" => Some(SecurityDecisionAction::Allow),
        "ask" => Some(SecurityDecisionAction::Ask),
        "block" => Some(SecurityDecisionAction::Block),
        "rewrite" => Some(SecurityDecisionAction::Rewrite),
        "throttle" => Some(SecurityDecisionAction::Throttle),
        _ => None,
    }
}

fn mcp_security_event_id(
    trace_id: Option<&str>,
    server_name: &str,
    tool_name: &str,
    request_id: Option<&str>,
    timestamp_unix_nanos: u128,
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(trace_id.unwrap_or("").as_bytes());
    hasher.update(server_name.as_bytes());
    hasher.update(tool_name.as_bytes());
    if let Some(request_id) = request_id {
        hasher.update(request_id.as_bytes());
    }
    hasher.update(&timestamp_unix_nanos.to_le_bytes());
    let hash = hasher.finalize().to_hex().to_string();
    format!("mcp-{}", &hash[..16])
}

#[cfg(test)]
mod tests;
