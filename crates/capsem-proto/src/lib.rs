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

pub mod handshake;
pub mod ipc;
pub mod metrics;
pub mod poll;

pub use handshake::{HandshakeError, Hello};

use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// Maximum size of a single control message frame (256KB).
/// Generous buffer for large payloads like CA bundles and file writes.
pub const MAX_FRAME_SIZE: u32 = 262_144;

/// Maximum number of env vars allowed during boot handshake.
pub const MAX_BOOT_ENV_VARS: usize = 128;

/// Maximum number of files allowed during boot handshake.
pub const MAX_BOOT_FILES: usize = 64;

/// Wire-protocol version for the bincode IPC channel and the vsock
/// control bridge. Bumped on any breaking change to
/// `{ServiceToProcess, ProcessToService, HostToGuest, GuestToHost}` or
/// to the framing of either transport.
///
/// `1` since the Hello handshake (W3) added Frame<T> wrapping to every
/// bincode channel and a typed Hello frame to the vsock control port.
/// Pre-W3 binaries fail decode within 1 second.
///
/// `2` adds the S07/S12 live metrics snapshot IPC contract.
pub const PROTOCOL_VERSION: u16 = 2;

/// FNV-1a 64 hash of the protocol enum source bytes (lib.rs + ipc.rs +
/// handshake.rs). Computed by `build.rs`. Detects "I added a variant in
/// the middle without bumping PROTOCOL_VERSION" -- silent re-numbering of
/// bincode variants -- which is exactly the bug that motivated this
/// sprint.
pub const SCHEMA_HASH: u64 = include!(concat!(env!("OUT_DIR"), "/schema_hash.txt"));

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
/// vsock port for guest lifecycle commands (shutdown/suspend from capsem-sysutil).
pub const VSOCK_PORT_LIFECYCLE: u32 = 5004;
/// vsock port for exec output (direct child process stdout from guest).
pub const VSOCK_PORT_EXEC: u32 = 5005;
/// vsock port for kernel audit stream (execve events from auditd via guest agent).
pub const VSOCK_PORT_AUDIT: u32 = 5006;
/// vsock port for the DNS proxy (T3): the guest agent's `capsem-dns-proxy`
/// listener forwards each DNS query to the host's hickory-backed handler
/// over an `rmp-serde` length-framed envelope.
pub const VSOCK_PORT_DNS_PROXY: u32 = 5007;

// ---------------------------------------------------------------------------
// Framed MCP transport (MITM MCP unification T0 wire gate)
// ---------------------------------------------------------------------------

/// Magic marker for framed MCP-over-vsock payloads on the MITM port.
pub const MCP_FRAME_MAGIC: u16 = 0x4d43; // "MC"
/// Version for the framed MCP transport envelope.
pub const MCP_FRAME_VERSION: u8 = 1;
/// Fixed header length after the `u32 total_frame_len_be` prefix.
pub const MCP_FRAME_HEADER_LEN: u8 = 16;
/// Notifications reserve `stream_id=0` and set this flag.
pub const MCP_FRAME_FLAG_NOTIFICATION: u16 = 0x0001;
/// Maximum MCP frame body size after the four-byte length prefix.
pub const MCP_FRAME_MAX_SIZE: usize = 1_052_672;
/// Maximum per-frame process attribution length.
pub const MCP_FRAME_MAX_PROCESS_NAME_LEN: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpFrame {
    pub stream_id: u32,
    pub flags: u16,
    pub process_name: String,
    pub payload: Vec<u8>,
}

impl McpFrame {
    pub fn is_notification(&self) -> bool {
        self.stream_id == 0 && self.flags & MCP_FRAME_FLAG_NOTIFICATION != 0
    }
}

/// Encode a framed MCP payload as:
/// `[u32 total_len_be][fixed header][process_name bytes][payload bytes]`.
pub fn encode_mcp_frame(
    stream_id: u32,
    flags: u16,
    process_name: &str,
    payload: &[u8],
) -> Result<Vec<u8>> {
    validate_mcp_frame_stream_flags(stream_id, flags)?;

    let process_name_bytes = process_name.as_bytes();
    if process_name_bytes.len() > MCP_FRAME_MAX_PROCESS_NAME_LEN {
        bail!(
            "MCP process name too long: {} bytes",
            process_name_bytes.len()
        );
    }

    let total_len = MCP_FRAME_HEADER_LEN as usize + process_name_bytes.len() + payload.len();
    if total_len > MCP_FRAME_MAX_SIZE {
        bail!("MCP frame too large: {total_len} bytes");
    }

    let process_name_len: u16 = process_name_bytes
        .len()
        .try_into()
        .context("MCP process name length overflow")?;
    let payload_len: u32 = payload
        .len()
        .try_into()
        .context("MCP payload length overflow")?;
    let total_len_u32: u32 = total_len.try_into().context("MCP frame length overflow")?;

    let mut out = Vec::with_capacity(total_len + 4);
    out.extend_from_slice(&total_len_u32.to_be_bytes());
    out.extend_from_slice(&MCP_FRAME_MAGIC.to_be_bytes());
    out.push(MCP_FRAME_VERSION);
    out.push(MCP_FRAME_HEADER_LEN);
    out.extend_from_slice(&stream_id.to_be_bytes());
    out.extend_from_slice(&flags.to_be_bytes());
    out.extend_from_slice(&process_name_len.to_be_bytes());
    out.extend_from_slice(&payload_len.to_be_bytes());
    out.extend_from_slice(process_name_bytes);
    out.extend_from_slice(payload);
    Ok(out)
}

fn validate_mcp_frame_stream_flags(stream_id: u32, flags: u16) -> Result<()> {
    let reserved_flags = flags & !MCP_FRAME_FLAG_NOTIFICATION;
    if reserved_flags != 0 {
        bail!("reserved MCP frame flags set: 0x{reserved_flags:04x}");
    }

    let notification = flags & MCP_FRAME_FLAG_NOTIFICATION != 0;
    match (stream_id, notification) {
        (0, true) => Ok(()),
        (0, false) => bail!("MCP stream id 0 is reserved for notifications"),
        (_, true) => bail!("MCP notification flag requires stream id 0"),
        (_, false) => Ok(()),
    }
}

/// Quick classifier used by the MITM first-byte sniff after process metadata
/// has been stripped.
pub fn looks_like_mcp_frame_prefix(buf: &[u8]) -> bool {
    if buf.len() < 6 {
        return false;
    }
    let total_len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if !(MCP_FRAME_HEADER_LEN as usize..=MCP_FRAME_MAX_SIZE).contains(&total_len) {
        return false;
    }
    u16::from_be_bytes([buf[4], buf[5]]) == MCP_FRAME_MAGIC
}

/// Decode the frame body after the four-byte total length prefix.
pub fn decode_mcp_frame_body(body: &[u8]) -> Result<McpFrame> {
    if body.len() < MCP_FRAME_HEADER_LEN as usize {
        bail!("MCP frame body too short: {} bytes", body.len());
    }
    if body.len() > MCP_FRAME_MAX_SIZE {
        bail!("MCP frame body too large: {} bytes", body.len());
    }

    let magic = u16::from_be_bytes([body[0], body[1]]);
    if magic != MCP_FRAME_MAGIC {
        bail!("invalid MCP frame magic: 0x{magic:04x}");
    }
    let version = body[2];
    if version != MCP_FRAME_VERSION {
        bail!("unsupported MCP frame version: {version}");
    }
    let header_len = body[3];
    if header_len != MCP_FRAME_HEADER_LEN {
        bail!("invalid MCP frame header length: {header_len}");
    }

    let stream_id = u32::from_be_bytes([body[4], body[5], body[6], body[7]]);
    let flags = u16::from_be_bytes([body[8], body[9]]);
    validate_mcp_frame_stream_flags(stream_id, flags)?;

    let process_name_len = u16::from_be_bytes([body[10], body[11]]) as usize;
    let payload_len = u32::from_be_bytes([body[12], body[13], body[14], body[15]]) as usize;
    if process_name_len > MCP_FRAME_MAX_PROCESS_NAME_LEN {
        bail!("MCP process name too long: {process_name_len} bytes");
    }

    let expected = MCP_FRAME_HEADER_LEN as usize + process_name_len + payload_len;
    if body.len() != expected {
        bail!(
            "invalid MCP frame length: body={} expected={expected}",
            body.len()
        );
    }

    let process_start = MCP_FRAME_HEADER_LEN as usize;
    let payload_start = process_start + process_name_len;
    let process_name = std::str::from_utf8(&body[process_start..payload_start])
        .context("MCP process name is not UTF-8")?
        .to_string();
    let payload = body[payload_start..].to_vec();

    Ok(McpFrame {
        stream_id,
        flags,
        process_name,
        payload,
    })
}

// ---------------------------------------------------------------------------
// Message types
// ---------------------------------------------------------------------------

/// Commands sent from host to guest over vsock:5000.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "t", content = "d", rename_all = "lowercase")]
pub enum HostToGuest {
    // -- Boot --
    /// Clock sync at boot (first message after Ready).
    /// `traceparent` is the W3C Trace Context header for this VM's
    /// boot operation (W5). The guest agent reads it on receipt and
    /// stamps every subsequent log line with the trace_id, so a
    /// failure during boot (kernel panic, init script error) appears
    /// in the unified timeline alongside the host-side spans.
    /// Empty string means "no trace context" (legacy hosts).
    BootConfig {
        epoch_secs: u64,
        #[serde(default)]
        traceparent: String,
    },
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
    // -- Reliability --
    /// Receipt confirmation for an ackable `GuestToHost` response
    /// (ExecDone, FileOpDone, FileContent, Error). Symmetric counterpart
    /// to `GuestToHost::Ack`: the host bridge emits this immediately on
    /// receipt of an ackable guest response; the agent holds every
    /// outbound ackable response in a pending map keyed by `id` and
    /// re-sends on every fresh control conn until the matching
    /// `AckReply` arrives. Closes the bidirectional silent-drop hole
    /// on the guest->host return path (Apple VZ post-restoreState
    /// pattern: write returns success, bytes never propagate).
    AckReply { id: u64 },
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

/// One DNS query forwarded from the guest agent to the host DNS handler.
///
/// `raw` is the wire-format DNS query bytes -- the agent never parses
/// them, so the host (which already speaks DNS via `hickory-proto`)
/// stays the single decoder. `proto` lets the host distinguish UDP
/// queries (datagram, single-shot) from TCP queries (length-prefixed
/// stream, may carry larger answers like AXFR / TXT) so the response
/// can match. `process_name` is reserved for T3.3 telemetry --
/// looking up the source process for a UDP DNS query at agent time
/// is racy (transient sockets), so we ship `None` for now and let the
/// host fold it in later if `/proc/net/udp` correlation becomes
/// reliable enough to bother.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DnsRequest {
    pub raw: Vec<u8>,
    /// "udp" or "tcp" -- the source-side transport, NOT the path used
    /// to reach the upstream nameserver (which is always UDP today).
    pub proto: String,
    #[serde(default)]
    pub process_name: Option<String>,
}

/// One DNS answer flowing host -> guest agent over the DNS vsock port.
///
/// `raw` is the wire-format response (synthetic NXDOMAIN, synthetic
/// SERVFAIL, or upstream-forwarded answer). `decision` mirrors
/// `capsem_logger::events::Decision::as_str()` ("allowed", "denied",
/// "error") so the agent can log the outcome without depending on the
/// logger crate. `rcode` is the DNS response code (0 / 2 / 3) so a
/// future agent-side metric can distinguish NXDOMAIN-on-block from
/// NXDOMAIN-from-upstream without re-parsing the wire bytes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DnsResponse {
    pub raw: Vec<u8>,
    pub decision: String,
    pub rcode: u16,
}

/// Encode a `DnsRequest` into a length-prefixed RMP frame.
pub fn encode_dns_request(req: &DnsRequest) -> Result<Vec<u8>> {
    let payload = rmp_serde::to_vec_named(req).context("failed to encode DnsRequest")?;
    let len = payload.len() as u32;
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&len.to_be_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

/// Decode a `DnsRequest` from an RMP payload (without the length prefix).
pub fn decode_dns_request(payload: &[u8]) -> Result<DnsRequest> {
    rmp_serde::from_slice(payload).context("failed to decode DnsRequest")
}

/// Encode a `DnsResponse` into a length-prefixed RMP frame.
pub fn encode_dns_response(resp: &DnsResponse) -> Result<Vec<u8>> {
    let payload = rmp_serde::to_vec_named(resp).context("failed to encode DnsResponse")?;
    let len = payload.len() as u32;
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&len.to_be_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

/// Decode a `DnsResponse` from an RMP payload (without the length prefix).
pub fn decode_dns_response(payload: &[u8]) -> Result<DnsResponse> {
    rmp_serde::from_slice(payload).context("failed to decode DnsResponse")
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
    // -- Reliability --
    /// Receipt confirmation for an ackable `HostToGuest` (Exec, FileWrite,
    /// FileRead, FileDelete). Sent by the agent immediately on read of
    /// such a message, *before* processing it. The host bridge holds
    /// every outbound ackable message in a pending map keyed by `id` and
    /// re-sends on the next fresh control conn until the matching `Ack`
    /// arrives -- this is the protocol-level cover for Apple VZ's
    /// post-restoreState silent-drop pattern (write returns success, the
    /// bytes never propagate, the original watchdog could only guess
    /// from a timer). Agent dedup ensures a re-sent message that
    /// actually did land twice doesn't double-execute.
    Ack { id: u64 },
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
// Frame-shape detector
// ---------------------------------------------------------------------------

/// Returns true if `data` starts with bytes that look like a `to_vec_named`
/// adjacently-tagged enum frame produced by [`encode_host_msg`] /
/// [`encode_guest_msg`] (the `HostToGuest` / `GuestToHost` envelopes use
/// `#[serde(tag = "t", content = "d")]`).
///
/// Frame shapes:
/// - Unit variants (e.g. `Pong`, `BootConfigDone`):
///   `0x81 0xa1 't' 0xa? <variant_name>` -- fixmap[1].
/// - Variants with payload (e.g. `BootConfig { epoch_secs }`):
///   `0x82 0xa1 't' 0xa? <variant_name> 0xa1 'd' ...` -- fixmap[2].
///
/// Use this from any sink that forwards raw guest output to a tty so a
/// stray IPC frame leaks loudly into telemetry instead of silently into
/// the user's terminal. Targets the start-of-buffer case only;
/// MessagePack bytes appearing in the middle of legitimate file content
/// (e.g. `cat msgpack-blob.bin`) are not a leak.
///
/// Tested in `crates/capsem/src/shell_exit/tests.rs` against every variant
/// of both envelopes.
pub fn looks_like_ipc_frame(data: &[u8]) -> bool {
    data.len() >= 4
        && (data[0] == 0x81 || data[0] == 0x82)
        && data[1] == 0xa1
        && data[2] == b't'
        && (0xa0..=0xbf).contains(&data[3])
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

    let ws_resolved = workspace_root.canonicalize().with_context(|| {
        format!(
            "cannot canonicalize workspace root: {}",
            workspace_root.display()
        )
    })?;

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
