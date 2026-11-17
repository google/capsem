//! Purge intent identifies a stored value; it never carries a copy of that value.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentVariablePurgeExecution {
    pub purge_id: String,
    pub timestamp_unix_ms: i64,
}

impl EnvironmentVariablePurgeExecution {
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        let valid = uuid::Uuid::parse_str(&self.purge_id).is_ok_and(|id| {
            id.to_string() == self.purge_id
                && (1..=8).contains(&id.get_version_num())
                && id.get_variant() == uuid::Variant::RFC4122
        });
        if !valid || self.timestamp_unix_ms < 0 {
            return Err("invalid variable purge execution");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedEnvironmentVariablePurge {
    pub purge_id: String,
    pub environment_id: String,
    pub source_revision_id: String,
    pub variable_name: String,
    pub timestamp_unix_ms: i64,
    pub actor: String,
}

impl PreparedEnvironmentVariablePurge {
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        let id = |value: &str| {
            uuid::Uuid::parse_str(value).is_ok_and(|id| {
                id.to_string() == value
                    && (1..=8).contains(&id.get_version_num())
                    && id.get_variant() == uuid::Variant::RFC4122
            })
        };
        let name = self.variable_name.as_bytes();
        if !id(&self.purge_id)
            || !id(&self.environment_id)
            || !id(&self.source_revision_id)
            || self.timestamp_unix_ms < 0
            || self.actor != "operator"
            || name.len() > capsem_proto::ENVIRONMENT_MAX_VARIABLE_NAME_BYTES
            || !name
                .first()
                .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
            || !name.iter().all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
            || capsem_proto::validate_env_key(&self.variable_name).is_err()
        {
            return Err("invalid operator variable purge intent");
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentVariablePurgeState {
    Prepared,
    Rewriting,
    Committed,
    ReclaimPending,
    Failed,
}
impl EnvironmentVariablePurgeState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Rewriting => "rewriting",
            Self::Committed => "committed",
            Self::ReclaimPending => "reclaim_pending",
            Self::Failed => "failed",
        }
    }
    pub fn parse_str(value: &str) -> Option<Self> {
        [
            Self::Prepared,
            Self::Rewriting,
            Self::Committed,
            Self::ReclaimPending,
            Self::Failed,
        ]
        .into_iter()
        .find(|state| state.as_str() == value)
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentVariablePurgeRecord {
    pub cursor: i64,
    pub intent: PreparedEnvironmentVariablePurge,
    pub state: EnvironmentVariablePurgeState,
    pub old_generation_id: Option<[u8; 16]>,
    pub new_generation_id: Option<[u8; 16]>,
    pub failure_reason: Option<String>,
    pub revision_count: i64,
    pub request_count: i64,
}
impl std::fmt::Debug for EnvironmentVariablePurgeRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvironmentVariablePurgeRecord")
            .field("cursor", &self.cursor)
            .field("purge_id", &self.intent.purge_id)
            .field("state", &self.state)
            .field("revision_count", &self.revision_count)
            .field("request_count", &self.request_count)
            .finish_non_exhaustive()
    }
}

impl EnvironmentVariablePurgeRecord {
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        if self.cursor <= 0
            || self.intent.validate().is_err()
            || self.revision_count <= 0
            || self.request_count < 0
            || self.revision_count.checked_add(self.request_count).is_none()
            || self.old_generation_id.is_some() != self.new_generation_id.is_some()
            || self
                .old_generation_id
                .zip(self.new_generation_id)
                .is_some_and(|(old, new)| old == new)
            || self
                .failure_reason
                .as_ref()
                .is_some_and(|reason| reason.trim().is_empty() || reason.contains('\0'))
            || (self.state == EnvironmentVariablePurgeState::Prepared && self.old_generation_id.is_some())
            || (matches!(
                self.state,
                EnvironmentVariablePurgeState::Rewriting
                    | EnvironmentVariablePurgeState::ReclaimPending
                    | EnvironmentVariablePurgeState::Committed
            ) && self.old_generation_id.is_none())
        {
            return Err("invalid environment purge metadata");
        }
        if self.old_generation_id == Some([0; 16])
            || self.new_generation_id == Some([0; 16])
            || self
                .failure_reason
                .as_ref()
                .is_some_and(|reason| reason.len() > capsem_proto::APPROVAL_CONTEXT_BYTES)
            || (self.state == EnvironmentVariablePurgeState::Failed && self.failure_reason.is_none())
        {
            return Err("invalid environment purge metadata");
        }
        Ok(())
    }
}
