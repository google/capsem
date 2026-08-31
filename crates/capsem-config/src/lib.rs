//! Pure configuration contracts for Capsem.
//!
//! This crate owns stable, serializable configuration identity without taking
//! dependencies on VM runtime, policy execution, telemetry storage, or MCP
//! transports.

pub mod model;
pub mod mcp;

/// True when a value has the broker-owned credential reference shape.
pub fn is_credential_reference(value: &str) -> bool {
    let Some(digest) = value.strip_prefix("credential:blake3:") else {
        return false;
    };
    digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
}
