//! What the host did to every session: the forensic record above the
//! per-session ledgers.
//!
//! A session ledger records what happened inside one VM. The host ledger
//! records what happened to it -- created from which image, started, forked,
//! stopped and why -- and what the service itself did. SQLite owns durable
//! ordering; a same-owner content hash chain supplies no independent authority.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostEventKind {
    SessionCreated,
    SessionStarted,
    SessionStopped,
    SessionRestarted,
    SessionForked,
    SessionDestroyed,
    SessionSuspended,
    SessionResumed,
    ImageAdmitted,
    ImageDenied,
    GrantMinted,
    GrantRevoked,
    AccessDenied,
    SettingsChanged,
    ServiceStarted,
    ServiceStopped,
    EnvironmentRevisionPrepared,
    EnvironmentRevisionCommitted,
    EnvironmentRevisionRecovered,
    EnvironmentRequestPrepared,
    EnvironmentRequestDecided,
    EnvironmentRequestInvalidated,
    EnvironmentRequestResultBound,
    EnvironmentReferenceChanged,
    EnvironmentApplicationChanged,
    EnvironmentVariablePurgePrepared,
}

impl HostEventKind {
    pub const ALL: [Self; 26] = [
        Self::SessionCreated,
        Self::SessionStarted,
        Self::SessionStopped,
        Self::SessionRestarted,
        Self::SessionForked,
        Self::SessionDestroyed,
        Self::SessionSuspended,
        Self::SessionResumed,
        Self::ImageAdmitted,
        Self::ImageDenied,
        Self::GrantMinted,
        Self::GrantRevoked,
        Self::AccessDenied,
        Self::SettingsChanged,
        Self::ServiceStarted,
        Self::ServiceStopped,
        Self::EnvironmentRevisionPrepared,
        Self::EnvironmentRevisionCommitted,
        Self::EnvironmentRevisionRecovered,
        Self::EnvironmentRequestPrepared,
        Self::EnvironmentRequestDecided,
        Self::EnvironmentRequestInvalidated,
        Self::EnvironmentRequestResultBound,
        Self::EnvironmentReferenceChanged,
        Self::EnvironmentApplicationChanged,
        Self::EnvironmentVariablePurgePrepared,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::SessionCreated => "session_created",
            Self::SessionStarted => "session_started",
            Self::SessionStopped => "session_stopped",
            Self::SessionRestarted => "session_restarted",
            Self::SessionForked => "session_forked",
            Self::SessionDestroyed => "session_destroyed",
            Self::SessionSuspended => "session_suspended",
            Self::SessionResumed => "session_resumed",
            Self::ImageAdmitted => "image_admitted",
            Self::ImageDenied => "image_denied",
            Self::GrantMinted => "grant_minted",
            Self::GrantRevoked => "grant_revoked",
            Self::AccessDenied => "access_denied",
            Self::SettingsChanged => "settings_changed",
            Self::ServiceStarted => "service_started",
            Self::ServiceStopped => "service_stopped",
            Self::EnvironmentRevisionPrepared => "environment_revision_prepared",
            Self::EnvironmentRevisionCommitted => "environment_revision_committed",
            Self::EnvironmentRevisionRecovered => "environment_revision_recovered",
            Self::EnvironmentRequestPrepared => "environment_request_prepared",
            Self::EnvironmentRequestInvalidated => "environment_request_invalidated",
            Self::EnvironmentRequestDecided => "environment_request_decided",
            Self::EnvironmentRequestResultBound => "environment_request_result_bound",
            Self::EnvironmentVariablePurgePrepared => "environment_variable_purge_prepared",
            Self::EnvironmentApplicationChanged => "environment_application_changed",
            Self::EnvironmentReferenceChanged => "environment_reference_changed",
        }
    }

    pub fn parse_str(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == value)
    }
}

/// One thing the host did. `detail` is kind-specific sparse MessagePack (a
/// stopped session's final counter snapshot, a fork's source, an admission's
/// selector), empty when the kind needs none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostEvent {
    pub timestamp_unix_ms: i64,
    pub kind: HostEventKind,
    #[serde(default)]
    pub session_id: Option<String>,
    pub actor: String,
    #[serde(default)]
    #[serde(with = "serde_bytes")]
    pub detail: Vec<u8>,
    #[serde(default)]
    pub trace_id: Option<String>,
}

#[cfg(test)]
mod tests;
