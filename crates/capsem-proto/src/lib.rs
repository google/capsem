//! Capsem protocol types for host/guest communication over vsock.
//!
//! Defines disjoint `HostToGuest` and `GuestToHost` message enums with
//! MessagePack framing. No platform-specific dependencies, so this crate
//! cross-compiles for both macOS host and aarch64-linux-musl guest.
//!
//! # Security invariant (RFC T14)
//!
//! The host only deserializes `GuestToHost`. The guest only deserializes
//! `HostToGuest`. This is enforced at the type level by having separate
//! encode/decode function pairs.

pub mod ipc;
pub mod poll;

use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

/// Maximum size of a single control message frame (256KB).
/// Generous buffer for large payloads like CA bundles and file writes.
pub const MAX_FRAME_SIZE: u32 = 262_144;

/// Maximum number of env vars allowed during boot handshake.
pub const MAX_BOOT_ENV_VARS: usize = 128;

/// Maximum number of files allowed during boot handshake.
pub const MAX_BOOT_FILES: usize = 64;

/// Maximum cumulative file bytes allowed during boot handshake (10MB).
pub const MAX_BOOT_FILE_BYTES: usize = 10_485_760;

/// Grace period (seconds) between SIGTERM and SIGKILL during shutdown.
/// capsem-sysutil derives its countdown from this (SHUTDOWN_GRACE_SECS + 1).
pub const SHUTDOWN_GRACE_SECS: u64 = 2;

/// Maximum length of an env var key.
pub const MAX_ENV_KEY_LEN: usize = 256;

/// Maximum length of an env var value (128KB).
pub const MAX_ENV_VALUE_LEN: usize = 131_072;

/// Env var names that are blocked during boot injection.
///
/// These are dangerous because they can hijack the dynamic linker, alter
/// shell behavior, or enable code injection before any user process runs.
/// Case-sensitive (Linux env vars are case-sensitive).
pub const BLOCKED_ENV_VARS: &[&str] = &[
    // Dynamic linker injection
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "LD_AUDIT",
    "LD_BIND_NOT",
    "LD_DEBUG",
    "LD_DYNAMIC_WEAK",
    "LD_PROFILE",
    "LD_SHOW_AUXV",
    "LD_USE_LOAD_BIAS",
    // Shell behavior hijacking
    "IFS",
    "BASH_ENV",
    "ENV",
    "CDPATH",
    "GLOBIGNORE",
    "SHELLOPTS",
    "BASHOPTS",
    "PROMPT_COMMAND",
    "PS4",
];

// ---------------------------------------------------------------------------
// Vsock port constants (shared between host and guest)
// ---------------------------------------------------------------------------

/// vsock port for structured control messages (resize, heartbeat, exec, file I/O).
pub const VSOCK_PORT_CONTROL: u32 = 5000;
/// vsock port for raw PTY byte streaming (stdin/stdout).
pub const VSOCK_PORT_TERMINAL: u32 = 5001;
/// vsock port for SNI proxy (HTTPS/HTTP traffic from guest).
pub const VSOCK_PORT_SNI_PROXY: u32 = 5002;
/// vsock port for MCP gateway (MCP tool calls from guest).
pub const VSOCK_PORT_MCP_GATEWAY: u32 = 5003;
/// vsock port for guest lifecycle commands (shutdown/suspend from capsem-sysutil).
pub const VSOCK_PORT_LIFECYCLE: u32 = 5004;
/// vsock port for exec output (direct child process stdout from guest).
pub const VSOCK_PORT_EXEC: u32 = 5005;
/// vsock port for kernel audit stream (execve events from auditd via guest agent).
pub const VSOCK_PORT_AUDIT: u32 = 5006;

// ---------------------------------------------------------------------------
// Message types
// ---------------------------------------------------------------------------

/// Commands sent from host to guest over vsock:5000.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "t", content = "d", rename_all = "lowercase")]
pub enum HostToGuest {
    // -- Boot --
    /// Clock sync at boot (first message after Ready).
    BootConfig { epoch_secs: u64 },
    /// Set a single environment variable in the guest.
    SetEnv { key: String, value: String },
    /// Signals that all boot-time env vars and files have been sent.
    BootConfigDone,
    // -- Terminal --
    /// Request terminal resize.
    Resize { cols: u16, rows: u16 },
    /// Execute command in guest PTY.
    Exec { id: u64, command: String },
    // -- Heartbeat --
    /// Liveness check + clock resync (handles Mac sleep drift).
    Ping { epoch_secs: u64 },
    // -- File operations (reserved) --
    /// Inject file into guest workspace.
    FileWrite {
        id: u64,
        path: String,
        data: Vec<u8>,
        mode: u32,
    },
    /// Request file content from guest.
    FileRead { id: u64, path: String },
    /// Delete file in guest workspace.
    FileDelete { id: u64, path: String },
    // -- Lifecycle --
    /// Graceful shutdown request.
    Shutdown,
    /// Quiescence: sync + fsfreeze before snapshot.
    PrepareSnapshot,
    /// Resume filesystem I/O after snapshot.
    Unfreeze,
}

/// A single boot timing measurement from the guest init script.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BootStage {
    pub name: String,
    pub duration_ms: u64,
}

/// A kernel audit record streamed from guest to host over vsock:5006.
///
/// Each record represents a single `execve` syscall captured by the kernel
/// audit subsystem. The guest agent tails auditd output, correlates multi-line
/// records by audit ID, and serializes them as MessagePack frames.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuditRecord {
    /// Microseconds since epoch (from kernel audit timestamp).
    pub timestamp_us: u64,
    /// Guest PID that called execve.
    pub pid: u32,
    /// Parent PID.
    pub ppid: u32,
    /// User ID.
    pub uid: u32,
    /// Executable path (e.g. "/usr/bin/python3").
    pub exe: String,
    /// Short command name (e.g. "python3").
    pub comm: Option<String>,
    /// Full command line (reconstructed from EXECVE record argv).
    pub argv: String,
    /// Working directory at exec time.
    pub cwd: Option<String>,
    /// TTY name (e.g. "pts/0") or None for background processes.
    pub tty: Option<String>,
    /// Kernel session ID (tty grouping).
    pub session_id: Option<u32>,
    /// Parent executable path (quick "bash spawned python" queries).
    pub parent_exe: Option<String>,
    /// Kernel audit event ID (for deduplication and tracing).
    pub audit_id: String,
}

/// Encode an `AuditRecord` into a length-prefixed RMP frame.
pub fn encode_audit_record(record: &AuditRecord) -> Result<Vec<u8>> {
    let payload = rmp_serde::to_vec_named(record).context("failed to encode AuditRecord")?;
    let len = payload.len() as u32;
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&len.to_be_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

/// Decode an `AuditRecord` from an RMP payload (without the length prefix).
pub fn decode_audit_record(payload: &[u8]) -> Result<AuditRecord> {
    rmp_serde::from_slice(payload).context("failed to decode AuditRecord")
}

/// Messages sent from guest to host over vsock:5000.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "t", content = "d", rename_all = "lowercase")]
pub enum GuestToHost {
    // -- Boot --
    /// Agent alive, waiting for boot config.
    Ready { version: String },
    /// Boot config applied, terminal ready.
    BootReady,
    /// Boot timing measurements from the guest init script.
    BootTiming { stages: Vec<BootStage> },
    // -- Terminal --
    /// Exec started: handshake on vsock exec port identifying the exec ID.
    ExecStarted { id: u64 },
    /// Command completed with exit code.
    ExecDone { id: u64, exit_code: i32 },
    // -- Heartbeat --
    /// Heartbeat response.
    Pong,
    // -- File telemetry (reserved) --
    /// Telemetry: file created in guest.
    FileCreated { path: String, size: u64 },
    /// Telemetry: file modified in guest.
    FileModified { path: String, size: u64 },
    /// Telemetry: file deleted in guest.
    FileDeleted { path: String },
    /// Response to FileRead.
    FileContent {
        id: u64,
        path: String,
        data: Vec<u8>,
    },
    /// Acknowledgment of a successful FileWrite or FileDelete.
    FileOpDone { id: u64 },
    /// Error encountered during a file operation or exec.
    Error { id: u64, message: String },
    // -- Lifecycle --
    /// Guest requests shutdown.
    ShutdownRequest,
    /// Guest requests suspend.
    SuspendRequest,
    /// Quiescence ack: filesystem frozen, safe to snapshot.
    SnapshotReady,
}

// ---------------------------------------------------------------------------
// Framing: [4-byte BE length][RMP payload]
// ---------------------------------------------------------------------------

/// Encode a `HostToGuest` message into a length-prefixed RMP frame.
pub fn encode_host_msg(msg: &HostToGuest) -> Result<Vec<u8>> {
    let payload = rmp_serde::to_vec_named(msg).context("failed to encode HostToGuest")?;
    let len = payload.len() as u32;
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&len.to_be_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

/// Decode a `HostToGuest` message from an RMP payload (without the length prefix).
pub fn decode_host_msg(payload: &[u8]) -> Result<HostToGuest> {
    rmp_serde::from_slice(payload).context("failed to decode HostToGuest")
}

/// Encode a `GuestToHost` message into a length-prefixed RMP frame.
pub fn encode_guest_msg(msg: &GuestToHost) -> Result<Vec<u8>> {
    let payload = rmp_serde::to_vec_named(msg).context("failed to encode GuestToHost")?;
    let len = payload.len() as u32;
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&len.to_be_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

/// Decode a `GuestToHost` message from an RMP payload (without the length prefix).
pub fn decode_guest_msg(payload: &[u8]) -> Result<GuestToHost> {
    rmp_serde::from_slice(payload).context("failed to decode GuestToHost")
}

/// Return the max allowed control frame size.
pub fn max_frame_size() -> u32 {
    MAX_FRAME_SIZE
}

// ---------------------------------------------------------------------------
// Boot handshake validation
// ---------------------------------------------------------------------------

/// Check if an env var name is blocked (exact match or `LD_` prefix).
pub fn is_blocked_env_var(key: &str) -> bool {
    if BLOCKED_ENV_VARS.contains(&key) {
        return true;
    }
    // Block any LD_ prefixed var not in the explicit list (catch-all for
    // future linker variables like LD_TRACE_LOADED_OBJECTS).
    if key.starts_with("LD_") {
        return true;
    }
    // Block bash function exports (BASH_FUNC_name%%)
    if key.starts_with("BASH_FUNC_") {
        return true;
    }
    false
}

/// Validate an env var key for boot injection.
///
/// Rejects: empty keys, keys containing `=` or NUL bytes, keys exceeding
/// `MAX_ENV_KEY_LEN`, and keys matching the blocklist.
pub fn validate_env_key(key: &str) -> Result<()> {
    if key.is_empty() {
        bail!("env var key is empty");
    }
    if key.contains('=') {
        bail!("env var key contains '=': {key:?}");
    }
    if key.contains('\0') {
        bail!("env var key contains NUL byte: {key:?}");
    }
    if key.len() > MAX_ENV_KEY_LEN {
        bail!(
            "env var key exceeds max length ({} > {MAX_ENV_KEY_LEN}): {key:?}",
            key.len()
        );
    }
    if is_blocked_env_var(key) {
        bail!("env var key is blocked: {key:?}");
    }
    Ok(())
}

/// Validate an env var value for boot injection.
///
/// Rejects: values containing NUL bytes, values exceeding `MAX_ENV_VALUE_LEN`.
pub fn validate_env_value(value: &str) -> Result<()> {
    if value.contains('\0') {
        bail!("env var value contains NUL byte");
    }
    if value.len() > MAX_ENV_VALUE_LEN {
        bail!(
            "env var value exceeds max length ({} > {MAX_ENV_VALUE_LEN})",
            value.len()
        );
    }
    Ok(())
}

/// Validate a file path for boot file injection.
///
/// Rejects: empty paths, paths containing NUL bytes, paths containing `..`.
pub fn validate_file_path(path: &str) -> Result<()> {
    if path.is_empty() {
        bail!("file path is empty");
    }
    if path.contains('\0') {
        bail!("file path contains NUL byte: {path:?}");
    }
    if path.contains("..") {
        bail!("file path contains '..': {path:?}");
    }
    Ok(())
}

/// Validate a file path for runtime file I/O inside the guest workspace.
///
/// Extends [`validate_file_path`] with symlink and containment checks:
/// 1. String-level validation (empty, NUL, `..`)
/// 2. Rejects paths that are themselves symlinks
/// 3. Canonicalizes the resolved path and verifies it stays within `workspace_root`
///
/// For new files (path does not exist yet), the parent directory is checked.
pub fn validate_file_path_safe(path: &str, workspace_root: &Path) -> Result<()> {
    validate_file_path(path)?;

    let full = Path::new(path);

    // Reject if the path itself is a symlink.
    if full
        .symlink_metadata()
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        bail!("path is a symlink: {path:?}");
    }

    let ws_resolved = workspace_root
        .canonicalize()
        .with_context(|| format!("cannot canonicalize workspace root: {}", workspace_root.display()))?;

    if full.exists() {
        // Existing path: canonicalize and check containment.
        let resolved = full
            .canonicalize()
            .with_context(|| format!("cannot canonicalize path: {path:?}"))?;
        if !resolved.starts_with(&ws_resolved) {
            bail!("path resolves outside workspace: {path:?}");
        }
    } else if let Some(parent) = full.parent() {
        // New file: canonicalize parent and check containment.
        if parent.exists() {
            let resolved_parent = parent
                .canonicalize()
                .with_context(|| format!("cannot canonicalize parent: {}", parent.display()))?;
            if !resolved_parent.starts_with(&ws_resolved) {
                bail!("parent resolves outside workspace: {path:?}");
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests;
