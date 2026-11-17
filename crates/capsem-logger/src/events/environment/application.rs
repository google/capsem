//! Acknowledged per-generation application facts, never runtime authority.
use super::{request::EnvironmentRequestTarget, EnvironmentRevisionParent};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentApplicationState {
    Pending,
    Applying,
    Applied,
    Failed,
}
impl EnvironmentApplicationState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Applying => "applying",
            Self::Applied => "applied",
            Self::Failed => "failed",
        }
    }
    pub fn parse_str(value: &str) -> Option<Self> {
        [Self::Pending, Self::Applying, Self::Applied, Self::Failed]
            .into_iter()
            .find(|state| state.as_str() == value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentApplicationComposition {
    pub boot: EnvironmentRevisionParent,
    pub live_policy: EnvironmentRevisionParent,
    pub desired: EnvironmentRevisionParent,
    pub corporate_policy_id: String,
    /// Core derives this from deferred settings; unequal revision IDs alone
    /// do not imply that workload configuration changed.
    pub pending_restart: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentSandboxReferences {
    pub boot: Option<EnvironmentRevisionParent>,
    pub live_policy: Option<EnvironmentRevisionParent>,
    pub desired: Option<EnvironmentRevisionParent>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentApplicationMutation {
    pub request_id: String,
    pub target: EnvironmentRequestTarget,
    pub expected_version: Option<i64>,
    pub expected_references: EnvironmentSandboxReferences,
    pub timestamp_unix_ms: i64,
    pub actor: String,
    pub state: EnvironmentApplicationState,
    pub composition: EnvironmentApplicationComposition,
    pub failure_reason: Option<String>,
}
impl std::fmt::Debug for EnvironmentApplicationMutation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvironmentApplicationMutation")
            .field("request_id", &self.request_id)
            .field("target", &self.target)
            .field("state", &self.state)
            .field("expected_version", &self.expected_version)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentApplicationRecord {
    pub cursor: i64,
    pub version: i64,
    pub request_id: String,
    pub target: EnvironmentRequestTarget,
    pub timestamp_unix_ms: i64,
    pub actor: String,
    pub state: EnvironmentApplicationState,
    pub composition: EnvironmentApplicationComposition,
    pub failure_reason: Option<String>,
}
impl std::fmt::Debug for EnvironmentApplicationRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvironmentApplicationRecord")
            .field("cursor", &self.cursor)
            .field("version", &self.version)
            .field("request_id", &self.request_id)
            .field("target", &self.target)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}
