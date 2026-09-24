//! Identity of one published active profile.
//!
//! The service writes a session's `active_profile.toml` and capsem-process
//! loads it on reload. Both sides name what they handled by the digest of the
//! exact bytes, so an acknowledgement proves which policy a VM applied rather
//! than merely that it applied one.

/// Digest of an active profile's serialized bytes, as `blake3:<hex>`.
pub fn active_profile_digest(bytes: &[u8]) -> String {
    format!("blake3:{}", blake3::hash(bytes).to_hex())
}
