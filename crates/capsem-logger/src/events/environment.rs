//! Typed machine-ledger revision records. Authority is supplied by the service's
//! verified caller boundary; these serializable records are evidence, not grants.
use capsem_config::environment::EnvironmentSpecification;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentRevisionParent {
    pub environment_id: String,
    pub revision_id: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedEnvironmentRevision {
    pub environment_id: String,
    pub revision_id: String,
    pub previous_revision_id: Option<String>,
    pub fork_parent: Option<EnvironmentRevisionParent>,
    pub timestamp_unix_ms: i64,
    pub actor: String,
    pub reason: String,
    pub request_id: Option<String>,
    pub payload_event_id: String,
    pub specification: Box<EnvironmentSpecification>,
}

impl std::fmt::Debug for PreparedEnvironmentRevision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedEnvironmentRevision")
            .field("environment_id", &self.environment_id)
            .field("revision_id", &self.revision_id)
            .field("specification", &self.specification)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentRevisionState {
    Committing,
    Committed,
    RecoveredUnapplied,
}

impl EnvironmentRevisionState {
    pub fn parse_str(value: &str) -> Option<Self> {
        match value {
            "committing" => Some(Self::Committing),
            "committed" => Some(Self::Committed),
            "recovered_unapplied" => Some(Self::RecoveredUnapplied),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Committing => "committing",
            Self::Committed => "committed",
            Self::RecoveredUnapplied => "recovered_unapplied",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentRevisionTransition {
    pub environment_id: String,
    pub revision_id: String,
    pub state: EnvironmentRevisionState,
    pub timestamp_unix_ms: i64,
    pub actor: String,
}

pub mod request;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentSandboxReferenceKind {
    Boot,
    LivePolicy,
    Desired,
}
impl EnvironmentSandboxReferenceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Boot => "sandbox_boot",
            Self::LivePolicy => "sandbox_live_policy",
            Self::Desired => "sandbox_desired",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentSandboxReferenceMutation {
    pub session_id: String,
    pub generation: String,
    pub kind: EnvironmentSandboxReferenceKind,
    pub expected: Option<EnvironmentRevisionParent>,
    pub revision: Option<EnvironmentRevisionParent>,
    pub timestamp_unix_ms: i64,
    pub actor: String,
}
