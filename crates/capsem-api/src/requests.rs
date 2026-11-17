//! Decisions are intent. Authentication and the observed request's current
//! eligibility remain mandatory service checks, independent of these DTOs.
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RequestDecision {
    Approve,
    Deny,
    Cancel,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RequestResolveRequest {
    /// Opaque observation returned by request info. A durable request uses its
    /// immutable payload event identity; a security ask uses its live generation.
    /// Neither value conveys approval authority.
    pub observed_revision: String,
    pub decision: RequestDecision,
    pub reason: String,
}
impl RequestResolveRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        let observation = self.observed_revision.as_bytes();
        let event_id = observation.len() == 12
            && observation
                .iter()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte));
        if !event_id && !crate::identity::canonical_uuid(&self.observed_revision) {
            return Err("request resolution requires the exact observed request revision or generation");
        }
        validate_reason(&self.reason)
    }
}
impl std::fmt::Debug for RequestResolveRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RequestResolveRequest")
            .field("decision", &self.decision)
            .finish_non_exhaustive()
    }
}

pub(crate) fn validate_reason(reason: &str) -> Result<(), &'static str> {
    if reason.trim().is_empty() || reason.len() > capsem_proto::APPROVAL_CONTEXT_BYTES || reason.contains('\0') {
        return Err("request reason must be nonempty bounded text without NUL");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
