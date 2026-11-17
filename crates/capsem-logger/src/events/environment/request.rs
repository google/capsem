//! Immutable request intent and resolution evidence, never caller credentials.
use super::EnvironmentRevisionParent;
use capsem_config::environment::EnvironmentDraft;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentRequestTarget {
    pub session_id: String,
    pub generation: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentRequestKind {
    Change,
    ImageUpdate,
    Promotion,
    Apply,
}
impl EnvironmentRequestKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Change => "change",
            Self::ImageUpdate => "image_update",
            Self::Promotion => "promotion",
            Self::Apply => "apply",
        }
    }
    pub fn parse_str(value: &str) -> Option<Self> {
        [Self::Change, Self::ImageUpdate, Self::Promotion, Self::Apply]
            .into_iter()
            .find(|kind| kind.as_str() == value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentRequestState {
    Pending,
    Approved,
    Denied,
    Cancelled,
    Expired,
    Stale,
}
impl EnvironmentRequestState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Denied => "denied",
            Self::Cancelled => "cancelled",
            Self::Expired => "expired",
            Self::Stale => "stale",
        }
    }
    pub fn parse_str(value: &str) -> Option<Self> {
        [
            Self::Pending,
            Self::Approved,
            Self::Denied,
            Self::Cancelled,
            Self::Expired,
            Self::Stale,
        ]
        .into_iter()
        .find(|state| state.as_str() == value)
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EnvironmentRequestPayload {
    Change {
        specification: Box<EnvironmentDraft>,
    },
    ImageUpdate {
        specification: Box<EnvironmentDraft>,
    },
    Promotion {
        source: EnvironmentRevisionParent,
    },
    Apply {
        revision: EnvironmentRevisionParent,
        targets: Vec<EnvironmentRequestTarget>,
    },
}

impl EnvironmentRequestPayload {
    /// Indexed reconstruction root, derived from the immutable payload.
    pub fn source_revision(&self) -> Option<&EnvironmentRevisionParent> {
        match self {
            Self::Promotion { source } => Some(source),
            Self::Apply { revision, .. } => Some(revision),
            Self::Change { .. } | Self::ImageUpdate { .. } => None,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Change { .. } => EnvironmentRequestKind::Change.as_str(),
            Self::ImageUpdate { .. } => EnvironmentRequestKind::ImageUpdate.as_str(),
            Self::Promotion { .. } => EnvironmentRequestKind::Promotion.as_str(),
            Self::Apply { .. } => EnvironmentRequestKind::Apply.as_str(),
        }
    }
}

impl std::fmt::Debug for EnvironmentRequestPayload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvironmentRequestPayload")
            .field("kind", &self.kind())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedEnvironmentRequest {
    pub request_id: String,
    pub environment_id: String,
    pub base_revision_id: Option<String>,
    pub requester: Option<EnvironmentRequestTarget>,
    pub timestamp_unix_ms: i64,
    pub expires_at_unix_ms: i64,
    pub actor: String,
    pub reason: String,
    pub payload_event_id: String,
    pub payload: Box<EnvironmentRequestPayload>,
}

impl std::fmt::Debug for PreparedEnvironmentRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedEnvironmentRequest")
            .field("request_id", &self.request_id)
            .field("environment_id", &self.environment_id)
            .field("payload", &self.payload)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentRequestDecisionKind {
    Approve,
    Deny,
    Cancel,
}

impl EnvironmentRequestDecisionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::Deny => "deny",
            Self::Cancel => "cancel",
        }
    }
    pub fn state(self) -> &'static str {
        match self {
            Self::Approve => EnvironmentRequestState::Approved.as_str(),
            Self::Deny => EnvironmentRequestState::Denied.as_str(),
            Self::Cancel => EnvironmentRequestState::Cancelled.as_str(),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentRequestDecision {
    pub request_id: String,
    pub timestamp_unix_ms: i64,
    pub actor: String,
    pub decision: EnvironmentRequestDecisionKind,
    pub reason: String,
}

impl std::fmt::Debug for EnvironmentRequestDecision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvironmentRequestDecision")
            .field("request_id", &self.request_id)
            .field("decision", &self.decision)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentRequestResult {
    pub request_id: String,
    pub result: EnvironmentRevisionParent,
    pub timestamp_unix_ms: i64,
    pub actor: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentRequestInvalidationKind {
    Expired,
    Stale,
}
impl EnvironmentRequestInvalidationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Expired => "expired",
            Self::Stale => "stale",
        }
    }
    pub fn parse_str(value: &str) -> Option<Self> {
        match value {
            "expired" => Some(Self::Expired),
            "stale" => Some(Self::Stale),
            _ => None,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentRequestInvalidation {
    pub request_id: String,
    pub timestamp_unix_ms: i64,
    pub actor: String,
    pub kind: EnvironmentRequestInvalidationKind,
    pub reason: String,
}
impl std::fmt::Debug for EnvironmentRequestInvalidation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvironmentRequestInvalidation")
            .field("request_id", &self.request_id)
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}
