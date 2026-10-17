//! Cryptographic provider ownership. Callers select protocol identities; this
//! crate selects and configures their accelerated implementations.
//! Existing SHA-256 and BLAKE3 identities are never interchangeable.

pub mod blake3 {
    //! The existing 256-bit BLAKE3 identity, including constant-time Hash equality.
    pub use ::blake3::{hash, Hash, Hasher};
}

#[cfg(feature = "sha256")]
mod sha256;
#[cfg(feature = "sha256")]
pub use sha256::{websocket_accept_digest, Sha256};

#[cfg(feature = "jwt")]
pub mod jwt;
#[cfg(feature = "random")]
pub mod random;
#[cfg(feature = "random")]
pub use random::uuid_v4;
#[cfg(feature = "secrets")]
pub mod secrets;
#[cfg(feature = "tls")]
pub mod tls;

pub mod verify;

#[cfg(test)]
mod tests;
