//! Human decisions are bound to a live session generation and original operation.
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SecurityAskStatus {
    Pending,
    Approved,
    Denied,
    Expired,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SecurityAskDecision {
    Approve,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SecurityAskResolveRequest {
    /// Exact boot generation returned by the list, never a display name.
    pub generation: String,
    pub decision: SecurityAskDecision,
}

impl SecurityAskResolveRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !crate::identity::canonical_uuid(&self.generation) {
            return Err("approval generation must be a canonical UUID");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SecurityAskRequest {
    pub ask_id: String,
    pub event_id: String,
    pub event_type: String,
    pub rule_id: String,
    pub rule_name: String,
    /// Bounded safe action facts, without request payloads or credentials.
    pub summary: String,
    pub status: SecurityAskStatus,
    pub created_at_unix_ms: i64,
    pub expires_at_unix_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_at_unix_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SecurityAskListResponse {
    pub vm_id: String,
    pub generation: String,
    /// Actionable live requests only; historical pending rows never count.
    pub pending_count: u32,
    pub requests: Vec<SecurityAskRequest>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SecurityAskListQuery {
    pub limit: Option<u16>,
}

#[cfg(test)]
mod tests;
