use crate::security_engine::{RuntimeSecurityEventType, SecurityBoundaryEvaluation, SecurityEnforcementDecision};

#[derive(Clone, Debug, Default)]
pub(super) struct SecurityBoundaryDecisionFields {
    pub(super) approval_event_id: Option<String>,
    pub(super) policy_mode: Option<String>,
    pub(super) policy_action: Option<String>,
    pub(super) policy_rule: Option<String>,
    pub(super) policy_reason: Option<String>,
}

impl SecurityBoundaryDecisionFields {
    pub(super) fn network_decision(&self) -> capsem_logger::Decision {
        match self.policy_action.as_deref() {
            Some("block") => capsem_logger::Decision::Denied,
            _ => capsem_logger::Decision::Allowed,
        }
    }

    pub(super) fn from_enforcement(decision: &SecurityEnforcementDecision) -> Self {
        Self {
            policy_mode: Some("enforce".to_string()),
            policy_action: Some(decision.action.as_str().to_string()),
            policy_rule: decision.rule_id.clone(),
            policy_reason: decision.reason.clone(),
            ..Default::default()
        }
    }

    pub(super) fn with_evaluation(self, evaluation: &SecurityBoundaryEvaluation) -> Self {
        Self {
            approval_event_id: self.approval_event_id.or_else(|| {
                if matches!(
                    evaluation.event.event_type,
                    RuntimeSecurityEventType::HttpRequest
                        | RuntimeSecurityEventType::McpToolCall
                        | RuntimeSecurityEventType::McpToolList
                        | RuntimeSecurityEventType::McpEvent
                ) {
                    evaluation.approval_event_id.as_ref().map(|id| id.as_str().to_owned())
                } else {
                    None
                }
            }),
            ..Self::from_enforcement(&evaluation.enforcement)
        }
    }

    pub(super) fn matched_rule(&self, fallback: String) -> String {
        self.policy_rule.clone().unwrap_or(fallback)
    }
}
