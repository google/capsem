//! Bounded, path-free human decisions for an original held operation.
//! The connected capability supplies session and generation, never the worker.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalOutcome {
    Approved,
    Denied,
    Expired,
    Cancelled,
    Unavailable,
}

impl ApprovalOutcome {
    pub fn is_allowed(self) -> bool {
        self == Self::Approved
    }
}

/// Only deliberately selected display facts cross this capability. No body,
/// command, environment, headers, query, tool arguments or credential material.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalRequest {
    pub ask_id: String,
    pub event_id: String,
    pub event_type: String,
    pub rule_id: String,
    pub rule_name: String,
    pub summary: String,
    pub held_bytes: u64,
}

impl ApprovalRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        let valid_id =
            |id: &str| id.len() == 12 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        let fields = [&self.event_type, &self.rule_id, &self.rule_name, &self.summary];
        if !valid_id(&self.ask_id)
            || !valid_id(&self.event_id)
            || fields
                .iter()
                .any(|field| field.is_empty() || field.chars().any(char::is_control))
            || fields.iter().map(|field| field.len()).sum::<usize>() > crate::APPROVAL_CONTEXT_BYTES
            || self.held_bytes > crate::APPROVAL_HELD_BYTES_PER_REQUEST as u64
        {
            return Err("invalid approval identity, context or held-byte bound");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalClientMessage {
    Begin { request_id: u64, request: ApprovalRequest },
    Cancel { request_id: u64 },
}

/// Trusted publication receipt for the original admitted continuation. This
/// conveys a finite wait, never permission to execute the operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalRegistration {
    pub ask_id: String,
    pub event_id: String,
    pub expires_at_unix_ms: i64,
    /// Remaining broker-owned monotonic lifetime when publication occurs.
    pub remaining_ms: u64,
}

impl ApprovalRegistration {
    pub fn validate(&self) -> Result<(), &'static str> {
        let valid_id =
            |id: &str| id.len() == 12 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !valid_id(&self.ask_id)
            || !valid_id(&self.event_id)
            || self.expires_at_unix_ms <= 0
            || self.remaining_ms == 0
            || self.remaining_ms > (crate::APPROVAL_TIMEOUT_SECONDS as u64) * 1000
        {
            return Err("invalid approval registration identity or expiry");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalServerMessage {
    Registered {
        request_id: u64,
        registration: ApprovalRegistration,
    },
    Resolved {
        request_id: u64,
        outcome: ApprovalOutcome,
    },
}

#[cfg(test)]
mod tests;
