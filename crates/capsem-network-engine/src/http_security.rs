use std::collections::BTreeMap;

use capsem_logger::Decision;
use capsem_security_engine::{
    AiAttributionScope, AiOriginKind, BlockResponse, Enforceability, HttpBodySecuritySubject,
    HttpSecuritySubject, RedactionState, ResolvedEventStep, ResolvedEventStepKind,
    ResolvedSecurityEvent, SecurityAction, SecurityDecision, SecurityDecisionAction, SecurityError,
    SecurityEvent, SecurityEventCommon, SourceEngine, StepStatus, RESOLVED_EVENT_SCHEMA_VERSION,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HttpIdentityContext {
    pub vm_id: Option<String>,
    pub session_id: Option<String>,
    pub profile_id: Option<String>,
    pub profile_revision: Option<String>,
    pub user_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpSecurityEventInput {
    pub event_id_seed: String,
    pub domain: String,
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    pub status_code: Option<u16>,
    pub request_headers: Option<String>,
    pub response_headers: Option<String>,
    pub request_bytes: u64,
    pub request_body_preview: Option<String>,
    pub response_bytes: Option<u64>,
    pub response_body_preview: Option<String>,
    pub port: u16,
    pub conn_type: String,
    pub identity: HttpIdentityContext,
    pub decision: Decision,
    pub matched_rule: Option<String>,
    pub policy_rule: Option<String>,
    pub policy_reason: Option<String>,
}

pub fn build_http_resolved_security_event(
    input: &HttpSecurityEventInput,
    timestamp_unix_ms: u64,
    trace_id: Option<String>,
) -> ResolvedSecurityEvent {
    let rule_id = input
        .policy_rule
        .clone()
        .or_else(|| input.matched_rule.clone());
    let reason = input
        .policy_reason
        .clone()
        .or_else(|| input.matched_rule.clone());
    let mut event = build_http_security_event(input, timestamp_unix_ms, trace_id);

    let mut steps = Vec::new();
    let final_action = match input.decision {
        Decision::Allowed | Decision::Redirected => {
            if let Some(rule_id) = rule_id.clone() {
                event.decision = Some(SecurityDecision {
                    action: SecurityDecisionAction::Allow,
                    rule: Some(rule_id.clone()),
                    pack_id: None,
                    reason: reason.clone(),
                    terminal: false,
                });
                steps.push(ResolvedEventStep {
                    kind: ResolvedEventStepKind::EnforcementMatch,
                    status: StepStatus::Matched,
                    rule_id: Some(rule_id),
                    pack_id: None,
                    message: reason.clone(),
                });
            }
            SecurityAction::Continue
        }
        Decision::Denied => {
            event.decision = Some(SecurityDecision {
                action: SecurityDecisionAction::Block,
                rule: rule_id.clone(),
                pack_id: None,
                reason: reason.clone(),
                terminal: true,
            });
            steps.push(ResolvedEventStep {
                kind: ResolvedEventStepKind::EnforcementMatch,
                status: StepStatus::Matched,
                rule_id: rule_id.clone(),
                pack_id: None,
                message: reason.clone(),
            });
            SecurityAction::Block(BlockResponse {
                reason_code: reason
                    .clone()
                    .unwrap_or_else(|| "network_request_denied".into()),
                rule_id,
            })
        }
        Decision::Error => {
            steps.push(ResolvedEventStep {
                kind: ResolvedEventStepKind::EnforcementMatch,
                status: StepStatus::Error,
                rule_id: rule_id.clone(),
                pack_id: None,
                message: reason.clone(),
            });
            SecurityAction::Error(SecurityError {
                code: "network_error".into(),
                message: reason.unwrap_or_else(|| "network request failed".into()),
            })
        }
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

pub fn build_http_security_event(
    input: &HttpSecurityEventInput,
    timestamp_unix_ms: u64,
    trace_id: Option<String>,
) -> SecurityEvent {
    let event_id = http_security_event_id_from_trace(input, trace_id.as_deref(), timestamp_unix_ms);
    SecurityEvent::http(
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
            vm_id: input.identity.vm_id.clone(),
            session_id: input.identity.session_id.clone(),
            profile_id: input.identity.profile_id.clone(),
            profile_revision: input.identity.profile_revision.clone(),
            profile_pack_ids: Vec::new(),
            enforcement_packs: Vec::new(),
            detection_packs: Vec::new(),
            user_id: input.identity.user_id.clone(),
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
            method: input.method.clone(),
            scheme: Some(http_scheme(input).into()),
            host: input.domain.clone(),
            port: Some(input.port),
            path: Some(input.path.clone()),
            query: input.query.clone(),
            url: Some(http_url(input)),
            path_class: http_path_class(&input.path),
            request_bytes: input.request_bytes,
            request_headers: parse_headers(input.request_headers.as_deref()),
            request_body: input
                .request_body_preview
                .clone()
                .map(HttpBodySecuritySubject::text),
            response_status: input.status_code,
            response_headers: parse_headers(input.response_headers.as_deref()),
            response_bytes: input.response_bytes,
            response_body: input
                .response_body_preview
                .clone()
                .map(HttpBodySecuritySubject::text),
        },
    )
}

pub fn build_http_response_security_event(
    input: &HttpSecurityEventInput,
    timestamp_unix_ms: u64,
    trace_id: Option<String>,
) -> SecurityEvent {
    let mut event = build_http_security_event(input, timestamp_unix_ms, trace_id);
    event.common.event_type = "http.response".into();
    event
}

fn http_scheme(input: &HttpSecurityEventInput) -> &'static str {
    if input.conn_type == "http-mitm" {
        "http"
    } else {
        "https"
    }
}

fn http_url(input: &HttpSecurityEventInput) -> String {
    match &input.query {
        Some(query) if !query.is_empty() => {
            format!(
                "{}://{}{}?{}",
                http_scheme(input),
                input.domain,
                input.path,
                query
            )
        }
        _ => format!("{}://{}{}", http_scheme(input), input.domain, input.path),
    }
}

fn http_path_class(path: &str) -> String {
    if path == "/" {
        "root".into()
    } else {
        path.trim_start_matches('/')
            .split('/')
            .next()
            .filter(|segment| !segment.is_empty())
            .unwrap_or("unknown")
            .to_owned()
    }
}

fn parse_headers(headers: Option<&str>) -> BTreeMap<String, Vec<String>> {
    let mut parsed = BTreeMap::new();
    let Some(headers) = headers else {
        return parsed;
    };
    for line in headers.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        if name.is_empty() {
            continue;
        }
        parsed
            .entry(name)
            .or_insert_with(Vec::new)
            .push(value.trim().to_string());
    }
    parsed
}

fn http_security_event_id_from_trace(
    input: &HttpSecurityEventInput,
    trace_id: Option<&str>,
    timestamp_unix_ms: u64,
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(input.event_id_seed.as_bytes());
    hasher.update(trace_id.unwrap_or("").as_bytes());
    hasher.update(input.domain.as_bytes());
    hasher.update(input.method.as_bytes());
    hasher.update(input.path.as_bytes());
    if let Some(query) = &input.query {
        hasher.update(query.as_bytes());
    }
    hasher.update(&timestamp_unix_ms.to_le_bytes());
    let hash = hasher.finalize().to_hex().to_string();
    format!("net-http-{}", &hash[..16])
}

#[cfg(test)]
mod tests;
