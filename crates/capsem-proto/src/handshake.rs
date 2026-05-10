//! Versioned handshake for IPC and vsock control channels.
//!
//! The bug pattern: capsem-service built before commit X and capsem-process
//! built after X talked past each other when an enum variant was added in
//! the middle of `ServiceToProcess`. Every IPC read silently swallowed the
//! decode error; suspend hung for 30 seconds with no log line that pointed
//! at the cause.
//!
//! With this handshake, the FIRST message on every newly-built channel is
//! a typed [`Hello`] carrying the protocol version, a compile-time schema
//! hash of the enum source, and the peer's binary identifier. A
//! [`Handshake`-mismatch][HandshakeError] log shows up in the JSON trace
//! within 1s; the support-bundle parser cross-references the two
//! `service.start` lines and points at the version skew immediately.

use serde::{Deserialize, Serialize};

/// First message on every typed IPC connection and every vsock control
/// connection. Sent by the *initiator* (service for IPC, guest for
/// vsock); the responder replies with its own Hello and both sides
/// cross-check.
///
/// The struct is itself versioned by adding fields with `#[serde(default)]`
/// at the end -- never reorder, never remove. A pre-handshake peer (no
/// Hello at all) trips the receive timeout in `negotiate()`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Hello {
    /// Bumped on any breaking change to the wire shape of the four
    /// protocol enums or the framing on either transport.
    pub version: u16,
    /// FNV-1a 64-bit hash of the protocol source bytes. Catches enum
    /// reordering / variant additions that don't bump `version`.
    pub schema_hash: u64,
    /// Free-form identifier ("capsem-service-1.0.1777", git sha, etc.).
    /// Logged on mismatch so the operator sees both peers at once.
    pub peer: String,
    /// W3C `traceparent` of the connection. Empty string for
    /// "no parent context" (top-of-tree initiator). W5 puts the value on
    /// the connection's *first* frame so per-message overhead is zero;
    /// per-message overrides ride `Frame::Msg.trace`.
    pub traceparent: String,
}

impl Hello {
    /// Construct a Hello carrying this binary's compile-time
    /// `PROTOCOL_VERSION` and `SCHEMA_HASH`.
    pub fn ours(peer: impl Into<String>, traceparent: impl Into<String>) -> Self {
        Self {
            version: crate::PROTOCOL_VERSION,
            schema_hash: crate::SCHEMA_HASH,
            peer: peer.into(),
            traceparent: traceparent.into(),
        }
    }
}

/// Errors from `negotiate()` and friends. Each carries enough context for
/// a single log line to identify both peers without needing a follow-up
/// query.
#[derive(thiserror::Error, Debug)]
pub enum HandshakeError {
    #[error("protocol version mismatch: ours={ours}, peer={peer}, peer_id={peer_id:?}")]
    Version {
        ours: u16,
        peer: u16,
        peer_id: String,
    },

    #[error(
        "schema hash mismatch (same version, incompatible enum layout): \
         ours={ours:016x}, peer={peer:016x}, peer_id={peer_id:?}"
    )]
    Schema {
        ours: u64,
        peer: u64,
        peer_id: String,
    },

    #[error("peer did not send Hello within {timeout_ms}ms (likely pre-handshake binary)")]
    Timeout { timeout_ms: u64 },

    #[error("transport error during handshake: {0}")]
    Io(#[from] std::io::Error),

    #[error("decode error during handshake: {0}")]
    Decode(String),
}

/// Verify that a peer's Hello is compatible with ours. Returns Ok(()) on
/// match, otherwise a typed error that the caller logs and surfaces.
pub fn verify(peer: &Hello) -> Result<(), HandshakeError> {
    if peer.version != crate::PROTOCOL_VERSION {
        return Err(HandshakeError::Version {
            ours: crate::PROTOCOL_VERSION,
            peer: peer.version,
            peer_id: peer.peer.clone(),
        });
    }
    if peer.schema_hash != crate::SCHEMA_HASH {
        return Err(HandshakeError::Schema {
            ours: crate::SCHEMA_HASH,
            peer: peer.schema_hash,
            peer_id: peer.peer.clone(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
