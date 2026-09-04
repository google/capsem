//! Dependency-light host foundations shared by Capsem binaries.
//!
//! This crate deliberately excludes VM, policy, database, HTTP, GUI, and MCP
//! runtime dependencies. It owns the process-level primitives that otherwise
//! force small binaries to compile all of `capsem-core`.

#[cfg(feature = "runtime")]
pub mod ipc_handshake;
#[cfg(feature = "runtime")]
pub mod log_layer;
#[cfg(feature = "runtime")]
pub mod paths;
#[cfg(feature = "runtime")]
pub mod poll;
pub mod proctable;
#[cfg(feature = "runtime")]
pub mod telemetry;
#[cfg(feature = "runtime")]
pub mod time;
#[cfg(feature = "runtime")]
pub mod uds;
#[cfg(feature = "runtime")]
pub mod unix;

#[cfg(test)]
pub(crate) static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
