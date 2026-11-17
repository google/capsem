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
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Change { .. } => "change",
            Self::ImageUpdate { .. } => "image_update",
            Self::Promotion { .. } => "promotion",
            Self::Apply { .. } => "apply",
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
            Self::Approve => "approved",
            Self::Deny => "denied",
            Self::Cancel => "cancelled",
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
