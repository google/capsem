//! Write classification and primary identity, shared by all producer paths.
use super::*;

impl WriteOp {
    pub fn kind(&self) -> &'static str {
        match self {
            WriteOp::TransportEvent(_) => "transport_event",
            WriteOp::NetEvent(_) => "net_event",
            WriteOp::ModelCall(_) => "model_call",
            WriteOp::McpCall(_) => "mcp_call",
            WriteOp::FileEvent(_) => "file_event",
            WriteOp::ExecEvent(_) => "exec_event",
            WriteOp::ExecEventComplete(_) => "exec_event_complete",
            WriteOp::AuditEvent(_) => "audit_event",
            WriteOp::DnsEvent(_) => "dns_event",
            WriteOp::SubstitutionEvent(_) => "substitution_event",
            WriteOp::SecurityRuleEvent(_) => "security_rule_event",
            WriteOp::SecurityAskEvent(_) => "security_ask_event",
            WriteOp::SecurityDecisionEvent(_) => "security_decision_event",
            WriteOp::ProfileMutationEvent(_) => "profile_mutation_event",
        }
    }

    /// Ensure a primary emitted event has a stable 12-lower-hex id before it
    /// reaches SQLite. Rule ledger rows already point at a triggering event and
    /// therefore must not mint their own id here.
    pub fn ensure_event_id(&mut self) -> Option<String> {
        match self {
            WriteOp::TransportEvent(event) => Some(event.event_id.clone()),
            WriteOp::NetEvent(event) => ensure_option_event_id(&mut event.event_id),
            WriteOp::ModelCall(event) => ensure_option_event_id(&mut event.event_id),
            WriteOp::McpCall(event) => ensure_option_event_id(&mut event.event_id),
            WriteOp::FileEvent(event) => ensure_option_event_id(&mut event.event_id),
            WriteOp::ExecEvent(event) => ensure_option_event_id(&mut event.event_id),
            WriteOp::AuditEvent(event) => ensure_option_event_id(&mut event.event_id),
            WriteOp::DnsEvent(event) => ensure_option_event_id(&mut event.event_id),
            WriteOp::SubstitutionEvent(event) => ensure_option_event_id(&mut event.event_id),
            WriteOp::SecurityRuleEvent(event) => Some(event.event_id.clone()),
            WriteOp::SecurityAskEvent(event) => Some(event.event_id.clone()),
            WriteOp::SecurityDecisionEvent(event) => Some(event.event_id.clone()),
            WriteOp::ProfileMutationEvent(event) => Some(event.mutation_id.clone()),
            WriteOp::ExecEventComplete(_) => None,
        }
    }

    pub fn event_id(&self) -> Option<&str> {
        match self {
            WriteOp::TransportEvent(event) => Some(event.event_id.as_str()),
            WriteOp::NetEvent(event) => event.event_id.as_deref(),
            WriteOp::ModelCall(event) => event.event_id.as_deref(),
            WriteOp::McpCall(event) => event.event_id.as_deref(),
            WriteOp::FileEvent(event) => event.event_id.as_deref(),
            WriteOp::ExecEvent(event) => event.event_id.as_deref(),
            WriteOp::AuditEvent(event) => event.event_id.as_deref(),
            WriteOp::DnsEvent(event) => event.event_id.as_deref(),
            WriteOp::SubstitutionEvent(event) => event.event_id.as_deref(),
            WriteOp::SecurityRuleEvent(event) => Some(event.event_id.as_str()),
            WriteOp::SecurityAskEvent(event) => Some(event.event_id.as_str()),
            WriteOp::SecurityDecisionEvent(event) => Some(event.event_id.as_str()),
            WriteOp::ProfileMutationEvent(event) => Some(event.mutation_id.as_str()),
            WriteOp::ExecEventComplete(_) => None,
        }
    }
}

fn ensure_option_event_id(event_id: &mut Option<String>) -> Option<String> {
    if event_id.is_none() {
        *event_id = Some(new_event_id());
    }
    event_id.clone()
}
