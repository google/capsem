use super::{SecurityActionError, SecurityAskStatus, SecurityEvent, SecurityEventId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityBoundaryEvaluation {
    /// Identity allocated before a proxy approval wait, retained for its
    /// completed protocol ledger row rather than minted again afterward.
    pub approval_event_id: Option<SecurityEventId>,
    pub event: SecurityEvent,
    pub enforcement: SecurityEnforcementDecision,
    pub matched_rule_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityEnforcementDecision {
    pub action: SecurityEnforcementAction,
    pub rule_id: Option<String>,
    pub rule_name: Option<String>,
    pub reason: Option<String>,
    pub ask_id: Option<SecurityEventId>,
}

impl SecurityEnforcementDecision {
    pub fn allow() -> Self {
        Self {
            action: SecurityEnforcementAction::Allow,
            rule_id: None,
            rule_name: None,
            reason: None,
            ask_id: None,
        }
    }

    pub fn is_allowed(&self) -> bool {
        matches!(self.action, SecurityEnforcementAction::Allow)
    }

    /// Apply an ask's resolution. It takes the three things it reads rather
    /// than a whole `SecurityAskEvent`: a recorded resolution keeps no payload.
    pub fn with_ask_resolution(
        &self,
        ask_id: &str,
        status: SecurityAskStatus,
        reason: Option<&str>,
    ) -> Result<Self, SecurityActionError> {
        if !matches!(self.action, SecurityEnforcementAction::Ask) {
            return Err(SecurityActionError::new(
                "only ask enforcement decisions can consume ask resolutions",
            ));
        }
        if self.ask_id.as_ref().map(SecurityEventId::as_str) != Some(ask_id) {
            return Err(SecurityActionError::new(format!(
                "ask resolution '{ask_id}' does not match enforcement ask id"
            )));
        }
        let action = match status {
            SecurityAskStatus::Pending => {
                return Err(SecurityActionError::new(format!("ask '{ask_id}' is still pending")))
            }
            SecurityAskStatus::Approved => SecurityEnforcementAction::Allow,
            SecurityAskStatus::Denied => SecurityEnforcementAction::Block,
        };
        Ok(Self {
            action,
            rule_id: self.rule_id.clone(),
            rule_name: self.rule_name.clone(),
            reason: reason.map(str::to_string).or_else(|| self.reason.clone()),
            ask_id: self.ask_id.clone(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityEnforcementAction {
    Allow,
    Ask,
    Block,
}
