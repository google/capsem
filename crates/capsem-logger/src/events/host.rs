//! What the host did to every session: the forensic record above the
//! per-session ledgers.
//!
//! A session ledger records what happened inside one VM. The host ledger
//! records what happened to it -- created from which image, started, forked,
//! stopped and why -- and what the service itself did. Each event carries the
//! hash of the one before it, so a removed, edited or reordered record breaks
//! the chain that `chain_hash` rebuilds.

use serde::{Deserialize, Serialize};

/// The hash the first event in a ledger chains from.
pub const GENESIS_HASH: [u8; 32] = [0; 32];

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
    SettingsChanged,
    ServiceStarted,
    ServiceStopped,
}

impl HostEventKind {
    pub const ALL: [Self; 15] = [
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
        Self::SettingsChanged,
        Self::ServiceStarted,
        Self::ServiceStopped,
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
            Self::SettingsChanged => "settings_changed",
            Self::ServiceStarted => "service_started",
            Self::ServiceStopped => "service_stopped",
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
    pub detail: Vec<u8>,
    #[serde(default)]
    pub trace_id: Option<String>,
}

/// The hash of `event` chained from `previous`: every field, length-prefixed
/// in a fixed order, so no serializer's choices can change it.
pub fn chain_hash(previous: &[u8; 32], event: &HostEvent) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(previous);
    hasher.update(&event.timestamp_unix_ms.to_le_bytes());
    let mut field = |bytes: Option<&[u8]>| match bytes {
        Some(bytes) => {
            hasher.update(&[1]);
            hasher.update(&(bytes.len() as u64).to_le_bytes());
            hasher.update(bytes);
        }
        None => {
            hasher.update(&[0]);
        }
    };
    field(Some(event.kind.as_str().as_bytes()));
    field(event.session_id.as_deref().map(str::as_bytes));
    field(Some(event.actor.as_bytes()));
    field(Some(&event.detail));
    field(event.trace_id.as_deref().map(str::as_bytes));
    *hasher.finalize().as_bytes()
}

#[cfg(test)]
mod tests;
