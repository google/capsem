//! Dependency-light host foundations shared by Capsem binaries.
//!
//! This crate deliberately excludes VM, policy, database, HTTP, GUI, and MCP
//! runtime dependencies. It owns the process-level primitives that otherwise
//! force small binaries to compile all of `capsem-core`.

#[cfg(feature = "runtime")]
pub mod ipc_channel;
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

#[cfg(test)]
pub(crate) fn isolated_test_command(program: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
    let coverage_profile = std::env::var_os("LLVM_PROFILE_FILE");
    let mut command = std::process::Command::new(program);
    command.env_clear();
    if let Some(profile) = coverage_profile {
        command.env("LLVM_PROFILE_FILE", profile);
    }
    command
}

#[cfg(test)]
mod tests;
