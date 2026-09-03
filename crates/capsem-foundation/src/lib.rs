//! Dependency-light host foundations shared by Capsem binaries.
//!
//! This crate deliberately excludes VM, policy, database, HTTP, GUI, and MCP
//! runtime dependencies. It owns the process-level primitives that otherwise
//! force small binaries to compile all of `capsem-core`.

pub mod ipc_handshake;
pub mod log_layer;
pub mod paths;
pub mod poll;
pub mod proctable;
pub mod telemetry;
pub mod time;
pub mod uds;

#[cfg(test)]
pub(crate) static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
