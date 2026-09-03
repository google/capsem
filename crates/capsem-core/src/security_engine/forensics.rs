//! Forensic JSON projections and tracing for security events and rule matches.

use super::*;

pub(super) fn logged_rule_action(action: SecurityRuleAction) -> LoggedRuleAction {
    match action {
        SecurityRuleAction::Allow => LoggedRuleAction::Allow,
        SecurityRuleAction::Ask => LoggedRuleAction::Ask,
        SecurityRuleAction::Block => LoggedRuleAction::Block,
        SecurityRuleAction::Preprocess => LoggedRuleAction::Preprocess,
        SecurityRuleAction::Rewrite => LoggedRuleAction::Rewrite,
        SecurityRuleAction::Postprocess => LoggedRuleAction::Postprocess,
    }
}

pub(super) fn logged_detection_level(level: Option<DetectionLevel>) -> LoggedDetectionLevel {
    match level {
        Some(DetectionLevel::Informational) => LoggedDetectionLevel::Informational,
        Some(DetectionLevel::Low) => LoggedDetectionLevel::Low,
        Some(DetectionLevel::Medium) => LoggedDetectionLevel::Medium,
        Some(DetectionLevel::High) => LoggedDetectionLevel::High,
        Some(DetectionLevel::Critical) => LoggedDetectionLevel::Critical,
        None => LoggedDetectionLevel::None,
    }
}

pub(super) fn compiled_rule_forensic_json(rule: &CompiledSecurityRule) -> serde_json::Value {
    json!({
        "rule_id": rule.rule_id,
        "provider": rule.provider,
        "namespace": rule.namespace,
        "rule_key": rule.rule_key,
        "name": rule.name,
        "rule_action": rule.action.as_str(),
        "match": rule.condition,
        "detection_level": rule
            .detection_level
            .map(|level| level.as_str())
            .unwrap_or("none"),
        "priority": rule.priority,
        "corp_locked": rule.corp_locked,
        "reason": rule.reason,
    })
}

pub(super) fn security_event_forensic_json(event: &SecurityEvent) -> serde_json::Value {
    json!({
        "event_type": event.event_type.as_str(),
        "credential_ref": event.credential_ref,
        "credential_observations": event.credential_observations.iter().map(|observation| {
            json!({
                "provider": observation.provider.as_str(),
                "source": observation.source,
                "event_type": observation.event_type,
                "trace_id": observation.trace_id,
                "context_json": observation.context_json,
                "credential_ref": observation.credential_ref(),
            })
        }).collect::<Vec<_>>(),
        "credential_injections": event.credential_injections.iter().map(|injection| {
            json!({
                "provider": injection.provider.map(|provider| provider.as_str()),
                "source": injection.source,
                "event_type": injection.event_type,
                "trace_id": injection.trace_id,
                "context_json": injection.context_json,
                "credential_ref": injection.credential_ref,
            })
        }).collect::<Vec<_>>(),
        "action_trace": event.action_trace.iter().map(|action| action.as_str()).collect::<Vec<_>>(),
        "decision": event.decision,
        "detections": event.detections,
        "plugin_executions": event.plugin_executions,
        "http_request": event.http_request.as_ref().map(http_request_forensic_json),
        "http": event.http,
        "dns": event.dns,
        "mcp": event.mcp,
        "model": event.model,
        "file": event.file,
        "process": event.process,
        "ip": event.ip,
        "tcp": event.tcp,
        "udp": event.udp,
    })
}

pub(super) fn http_request_forensic_json(request: &HttpRequestSecurityEvent) -> serde_json::Value {
    let headers = request
        .headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_string(),
                value.to_str().unwrap_or("<non-utf8>").to_string(),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();

    json!({
        "domain": request.domain,
        "ai_provider": request.ai_provider.map(|provider| provider.as_str()),
        "headers": headers,
        "query": request.query,
    })
}

pub(super) fn trace_runtime_security_event(event: &RuntimeSecurityEvent) {
    tracing::debug!(
        event_type = event.event_type.as_str(),
        event_family = event.event_family.as_str(),
        event_id = event.event_id.as_ref().map(|id| id.as_str()),
        credential_ref = event.credential_ref.as_deref(),
        trace_id = event.trace_id.as_deref(),
        "runtime security event emitted"
    );
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityRuleTraceLabels {
    pub rule_id: String,
    pub rule_name: String,
    pub rule_action: &'static str,
    pub rule_detection_level: &'static str,
    pub provider: String,
}

impl SecurityRuleTraceLabels {
    pub fn from_rule(rule: &CompiledSecurityRule) -> Self {
        Self {
            rule_id: rule.rule_id.clone(),
            rule_name: rule.name.clone(),
            rule_action: rule.action.as_str(),
            rule_detection_level: rule.detection_level.map(|level| level.as_str()).unwrap_or("none"),
            provider: rule.provider.clone(),
        }
    }
}

pub(super) fn trace_security_rule_match(event: &SecurityRuleEvent, rule: &CompiledSecurityRule) {
    let labels = SecurityRuleTraceLabels::from_rule(rule);
    tracing::debug!(
        event_id = event.event_id.as_str(),
        event_type = event.event_type.as_str(),
        trace_id = event.trace_id.as_deref(),
        rule_id = labels.rule_id.as_str(),
        rule_name = labels.rule_name.as_str(),
        rule_action = labels.rule_action,
        rule_detection_level = labels.rule_detection_level,
        provider = labels.provider.as_str(),
        "security rule matched"
    );
}

pub(super) fn logger_write_credential_ref(op: &WriteOp) -> Option<String> {
    match op {
        WriteOp::NetEvent(event) => event.credential_ref.clone(),
        WriteOp::ModelCall(event) => event.credential_ref.clone(),
        WriteOp::McpCall(event) => event.credential_ref.clone(),
        WriteOp::FileEvent(event) => event.credential_ref.clone(),
        WriteOp::ExecEvent(event) => event.credential_ref.clone(),
        WriteOp::ExecEventComplete(_) => None,
        WriteOp::AuditEvent(event) => event.credential_ref.clone(),
        WriteOp::DnsEvent(event) => event.credential_ref.clone(),
        WriteOp::SubstitutionEvent(event) => Some(event.substitution_ref.clone()),
        WriteOp::SecurityRuleEvent(event) => event.credential_ref.clone(),
        WriteOp::SecurityAskEvent(_) => None,
        WriteOp::SecurityDecisionEvent(event) => event.credential_ref.clone(),
        WriteOp::ProfileMutationEvent(_) => None,
    }
}

pub(super) fn logger_write_trace_id(op: &WriteOp) -> Option<String> {
    match op {
        WriteOp::NetEvent(event) => event.trace_id.clone(),
        WriteOp::ModelCall(event) => event.trace_id.clone(),
        WriteOp::McpCall(event) => event.trace_id.clone(),
        WriteOp::FileEvent(event) => event.trace_id.clone(),
        WriteOp::ExecEvent(event) => event.trace_id.clone(),
        WriteOp::ExecEventComplete(_) => None,
        WriteOp::AuditEvent(event) => event.trace_id.clone(),
        WriteOp::DnsEvent(event) => event.trace_id.clone(),
        WriteOp::SubstitutionEvent(event) => event.trace_id.clone(),
        WriteOp::SecurityRuleEvent(event) => event.trace_id.clone(),
        WriteOp::SecurityAskEvent(event) => event.trace_id.clone(),
        WriteOp::SecurityDecisionEvent(event) => event.trace_id.clone(),
        WriteOp::ProfileMutationEvent(event) => event.trace_id.clone(),
    }
}
