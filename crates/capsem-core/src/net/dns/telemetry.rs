//! Build a `DnsEvent` row from the handler's structured result + the
//! envelope the agent sent (T3.3). Pure function -- testable without
//! sqlite. Callers (vsock dispatch in `capsem-process`) push the event
//! into the `DbWriter` channel via `WriteOp::DnsEvent`.
//!
//! There's no "DnsTelemetryHook" struct because DNS doesn't need the
//! chunk-pipeline machinery the MITM proxy uses -- a DNS query is
//! single-shot bytes-in / bytes-out. Keeping this as a free function
//! lets the dispatch decide when (and whether) to record, without
//! coupling the handler to a `DbWriter`.

use std::net::IpAddr;
use std::time::SystemTime;

use capsem_logger::events::DnsEvent;
use capsem_security_engine::{
    AiAttributionScope, AiOriginKind, BlockResponse, DnsSecuritySubject, Enforceability,
    RedactionState, ResolvedEventStep, ResolvedEventStepKind, ResolvedSecurityEvent,
    SecurityAction, SecurityDecision, SecurityDecisionAction, SecurityError, SecurityEvent,
    SecurityEventCommon, SourceEngine, StepStatus, RESOLVED_EVENT_SCHEMA_VERSION,
};

use crate::net::dns::server::DnsHandlerResult;

/// Build a `DnsEvent` row for one query.
///
/// `result.query` is `None` when the input bytes failed to decode at
/// all -- in that case we fall back to "INVALID_DNS_BYTES" / qtype=0
/// / qclass=0 so the row still surfaces in `dns_events` and ops can
/// see "the agent sent us garbage" without losing the timestamp +
/// trace_id correlation.
pub fn build_dns_event(
    result: &DnsHandlerResult,
    source_proto: Option<&str>,
    process_name: Option<String>,
    trace_id: Option<String>,
) -> DnsEvent {
    let (qname, qtype, qclass) = match &result.query {
        Some(q) => (q.qname.clone(), q.qtype, q.qclass),
        None => ("INVALID_DNS_BYTES".to_string(), 0u16, 0u16),
    };

    DnsEvent {
        timestamp: SystemTime::now(),
        qname,
        qtype,
        qclass,
        rcode: result.rcode,
        decision: result.decision.as_str().to_string(),
        matched_rule: result.matched_rule.clone(),
        source_proto: source_proto.map(|s| s.to_string()),
        process_name,
        upstream_resolver_ms: result.upstream_resolver_ms,
        trace_id,
        policy_mode: result.policy_mode.clone(),
        policy_action: result.policy_action.clone(),
        policy_rule: result.policy_rule.clone(),
        policy_reason: result.policy_reason.clone(),
    }
}

/// Build the normalized Security Engine journal row for a DNS query result.
///
/// DNS enforcement still happens in the DNS handler today; this projection
/// makes that handler result visible through the canonical security event
/// ledger beside the legacy `dns_events` row.
pub fn build_dns_resolved_security_event(event: &DnsEvent) -> ResolvedSecurityEvent {
    let timestamp_unix_ms = event
        .timestamp
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let rule_id = event
        .policy_rule
        .clone()
        .or_else(|| event.matched_rule.clone());
    let reason = event
        .policy_reason
        .clone()
        .or_else(|| event.matched_rule.clone());
    let mut security_event = SecurityEvent::dns(
        SecurityEventCommon {
            event_id: dns_security_event_id(
                event.trace_id.as_deref(),
                &event.qname,
                event.qtype,
                event.qclass,
                timestamp_unix_ms,
            ),
            parent_event_id: None,
            stream_id: None,
            activity_id: None,
            sequence_no: None,
            source_engine: SourceEngine::Network,
            attribution_scope: AiAttributionScope::Vm,
            origin_kind: AiOriginKind::GuestNetwork,
            accounting_owner: None,
            enforceability: Enforceability::InlineBlockable,
            trace_id: event.trace_id.clone(),
            span_id: None,
            timestamp_unix_ms,
            vm_id: non_empty_env(crate::telemetry::CAPSEM_VM_ID_ENV),
            session_id: non_empty_env(crate::telemetry::CAPSEM_SESSION_ID_ENV),
            profile_id: non_empty_env(crate::telemetry::CAPSEM_PROFILE_ID_ENV),
            profile_revision: non_empty_env(crate::telemetry::CAPSEM_PROFILE_REVISION_ENV),
            profile_pack_ids: Vec::new(),
            enforcement_packs: Vec::new(),
            detection_packs: Vec::new(),
            user_id: non_empty_env(crate::telemetry::CAPSEM_USER_ID_ENV),
            process_id: None,
            parent_process_id: None,
            exec_id: None,
            turn_id: None,
            message_id: None,
            tool_call_id: None,
            mcp_call_id: None,
            event_type: "dns.request".into(),
            redaction_state: RedactionState::Raw,
        },
        DnsSecuritySubject {
            qname: event.qname.clone(),
            domain_class: dns_domain_class(&event.qname).into(),
        },
    );

    let decision_action = event
        .policy_action
        .as_deref()
        .and_then(dns_security_decision_action)
        .or_else(|| dns_security_decision_from_event_decision(&event.decision, rule_id.is_some()));

    if let Some(action) = decision_action {
        security_event.decision = Some(SecurityDecision {
            action,
            rule: rule_id.clone(),
            pack_id: None,
            reason: reason.clone(),
            terminal: matches!(
                action,
                SecurityDecisionAction::Ask
                    | SecurityDecisionAction::Block
                    | SecurityDecisionAction::Rewrite
                    | SecurityDecisionAction::Throttle
            ),
        });
    }

    let mut steps = Vec::new();
    if rule_id.is_some() || reason.is_some() || event.decision == "error" {
        steps.push(ResolvedEventStep {
            kind: ResolvedEventStepKind::EnforcementMatch,
            status: if event.decision == "error" {
                StepStatus::Error
            } else {
                StepStatus::Matched
            },
            rule_id: rule_id.clone(),
            pack_id: None,
            message: reason.clone(),
        });
    }

    let final_action = match event.decision.as_str() {
        "denied" => SecurityAction::Block(BlockResponse {
            reason_code: reason
                .clone()
                .unwrap_or_else(|| "dns_request_denied".into()),
            rule_id,
        }),
        "error" => SecurityAction::Error(SecurityError {
            code: "dns_error".into(),
            message: reason.unwrap_or_else(|| "DNS request failed".into()),
        }),
        _ => SecurityAction::Continue,
    };

    ResolvedSecurityEvent {
        schema_version: RESOLVED_EVENT_SCHEMA_VERSION,
        event: security_event,
        steps,
        plugin_transforms: Vec::new(),
        detection_findings: Vec::new(),
        final_action,
        emitter_results: Vec::new(),
    }
}

fn non_empty_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn dns_security_decision_action(action: &str) -> Option<SecurityDecisionAction> {
    match action {
        "allow" => Some(SecurityDecisionAction::Allow),
        "ask" => Some(SecurityDecisionAction::Ask),
        "block" => Some(SecurityDecisionAction::Block),
        "rewrite" => Some(SecurityDecisionAction::Rewrite),
        "throttle" => Some(SecurityDecisionAction::Throttle),
        _ => None,
    }
}

fn dns_security_decision_from_event_decision(
    decision: &str,
    has_rule: bool,
) -> Option<SecurityDecisionAction> {
    match decision {
        "allowed" if has_rule => Some(SecurityDecisionAction::Allow),
        "denied" => Some(SecurityDecisionAction::Block),
        _ => None,
    }
}

fn dns_domain_class(qname: &str) -> &'static str {
    if qname == "INVALID_DNS_BYTES" {
        return "invalid";
    }
    let normalized = qname.trim_end_matches('.').to_ascii_lowercase();
    if normalized == "localhost" || normalized.ends_with(".local") {
        return "local";
    }
    if normalized.parse::<IpAddr>().is_ok() {
        return "address";
    }
    "external"
}

fn dns_security_event_id(
    trace_id: Option<&str>,
    qname: &str,
    qtype: u16,
    qclass: u16,
    timestamp_unix_ms: u64,
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(trace_id.unwrap_or("").as_bytes());
    hasher.update(qname.as_bytes());
    hasher.update(&qtype.to_be_bytes());
    hasher.update(&qclass.to_be_bytes());
    hasher.update(&timestamp_unix_ms.to_be_bytes());
    format!("dns-{}", hasher.finalize().to_hex()[..16].to_string())
}

#[cfg(test)]
mod tests;
