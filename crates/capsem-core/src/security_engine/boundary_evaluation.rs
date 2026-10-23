use super::{SecurityEnforcementDecision, SecurityEvent, SecurityEventId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityBoundaryEvaluation {
    /// Identity allocated before a proxy approval wait, retained for its
    /// completed protocol ledger row rather than minted again afterward.
    pub approval_event_id: Option<SecurityEventId>,
    pub event: SecurityEvent,
    pub enforcement: SecurityEnforcementDecision,
    pub matched_rule_count: usize,
}
