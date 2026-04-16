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
    /// Liveness check.
    Ping,
    /// Heartbeat with clock sync.
    Heartbeat { epoch_secs: u64 },
    // -- File operations (reserved) --
    /// Inject file into guest workspace.
    FileWrite {
        path: String,
        data: Vec<u8>,
        mode: u32,
    },
    /// Request file content from guest.
    FileRead { id: u64, path: String },
    /// Delete file in guest workspace.
    FileDelete { path: String },
    // -- Lifecycle (reserved) --
    /// Graceful shutdown request.
    Shutdown,
}

/// A single boot timing measurement from the guest init script.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BootStage {
    pub name: String,
    pub duration_ms: u64,
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

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------
    // HostToGuest roundtrip
    // -------------------------------------------------------------------

    #[test]
    fn roundtrip_boot_config() {
        let msg = HostToGuest::BootConfig {
            epoch_secs: 1708800000,
        };
        let frame = encode_host_msg(&msg).unwrap();
        let len = u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]);
        assert!(len < MAX_FRAME_SIZE);
        let decoded = decode_host_msg(&frame[4..]).unwrap();
        match decoded {
            HostToGuest::BootConfig { epoch_secs } => {
                assert_eq!(epoch_secs, 1708800000);
            }
            other => panic!("expected BootConfig, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_set_env() {
        let msg = HostToGuest::SetEnv {
            key: "ANTHROPIC_API_KEY".into(),
            value: "sk-test-123".into(),
        };
        let frame = encode_host_msg(&msg).unwrap();
        let decoded = decode_host_msg(&frame[4..]).unwrap();
        match decoded {
            HostToGuest::SetEnv { key, value } => {
                assert_eq!(key, "ANTHROPIC_API_KEY");
                assert_eq!(value, "sk-test-123");
            }
            other => panic!("expected SetEnv, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_boot_config_done() {
        let msg = HostToGuest::BootConfigDone;
        let frame = encode_host_msg(&msg).unwrap();
        let decoded = decode_host_msg(&frame[4..]).unwrap();
        assert!(matches!(decoded, HostToGuest::BootConfigDone));
    }

    #[test]
    fn set_env_fits_in_frame() {
        // A 128KB env var value should fit in a single 256KB frame.
        let msg = HostToGuest::SetEnv {
            key: "LARGE_VAR".into(),
            value: "x".repeat(MAX_ENV_VALUE_LEN),
        };
        let frame = encode_host_msg(&msg).unwrap();
        let payload_len = frame.len() - 4;
        assert!(
            payload_len <= MAX_FRAME_SIZE as usize,
            "SetEnv payload is {payload_len} bytes, exceeds max {MAX_FRAME_SIZE}"
        );
    }

    #[test]
    fn roundtrip_resize() {
        let msg = HostToGuest::Resize {
            cols: 120,
            rows: 40,
        };
        let frame = encode_host_msg(&msg).unwrap();
        let decoded = decode_host_msg(&frame[4..]).unwrap();
        match decoded {
            HostToGuest::Resize { cols, rows } => {
                assert_eq!(cols, 120);
                assert_eq!(rows, 40);
            }
            other => panic!("expected Resize, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_exec() {
        let msg = HostToGuest::Exec {
            id: 42,
            command: "echo hello && ls -la".into(),
        };
        let frame = encode_host_msg(&msg).unwrap();
        let decoded = decode_host_msg(&frame[4..]).unwrap();
        match decoded {
            HostToGuest::Exec { id, command } => {
                assert_eq!(id, 42);
                assert_eq!(command, "echo hello && ls -la");
            }
            other => panic!("expected Exec, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_ping() {
        let msg = HostToGuest::Ping;
        let frame = encode_host_msg(&msg).unwrap();
        let decoded = decode_host_msg(&frame[4..]).unwrap();
        assert!(matches!(decoded, HostToGuest::Ping));
    }

    #[test]
    fn roundtrip_heartbeat() {
        let msg = HostToGuest::Heartbeat { epoch_secs: 123456789 };
        let frame = encode_host_msg(&msg).unwrap();
        let decoded = decode_host_msg(&frame[4..]).unwrap();
        match decoded {
            HostToGuest::Heartbeat { epoch_secs } => assert_eq!(epoch_secs, 123456789),
            other => panic!("expected Heartbeat, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_file_write() {
        let msg = HostToGuest::FileWrite {
            path: "/workspace/test.txt".into(),
            data: b"hello world".to_vec(),
            mode: 0o644,
        };
        let frame = encode_host_msg(&msg).unwrap();
        let decoded = decode_host_msg(&frame[4..]).unwrap();
        match decoded {
            HostToGuest::FileWrite { path, data, mode } => {
                assert_eq!(path, "/workspace/test.txt");
                assert_eq!(data, b"hello world");
                assert_eq!(mode, 0o644);
            }
            other => panic!("expected FileWrite, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_file_read() {
        let msg = HostToGuest::FileRead {
            id: 7,
            path: "/workspace/out.log".into(),
        };
        let frame = encode_host_msg(&msg).unwrap();
        let decoded = decode_host_msg(&frame[4..]).unwrap();
        match decoded {
            HostToGuest::FileRead { id, path } => {
                assert_eq!(id, 7);
                assert_eq!(path, "/workspace/out.log");
            }
            other => panic!("expected FileRead, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_file_delete() {
        let msg = HostToGuest::FileDelete {
            path: "/workspace/tmp".into(),
        };
        let frame = encode_host_msg(&msg).unwrap();
        let decoded = decode_host_msg(&frame[4..]).unwrap();
        match decoded {
            HostToGuest::FileDelete { path } => assert_eq!(path, "/workspace/tmp"),
            other => panic!("expected FileDelete, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_shutdown() {
        let msg = HostToGuest::Shutdown;
        let frame = encode_host_msg(&msg).unwrap();
        let decoded = decode_host_msg(&frame[4..]).unwrap();
        assert!(matches!(decoded, HostToGuest::Shutdown));
    }

    // -------------------------------------------------------------------
    // GuestToHost roundtrip
    // -------------------------------------------------------------------

    #[test]
    fn roundtrip_ready() {
        let msg = GuestToHost::Ready {
            version: "0.3.0".into(),
        };
        let frame = encode_guest_msg(&msg).unwrap();
        let decoded = decode_guest_msg(&frame[4..]).unwrap();
        match decoded {
            GuestToHost::Ready { version } => assert_eq!(version, "0.3.0"),
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_boot_ready() {
        let msg = GuestToHost::BootReady;
        let frame = encode_guest_msg(&msg).unwrap();
        let decoded = decode_guest_msg(&frame[4..]).unwrap();
        assert!(matches!(decoded, GuestToHost::BootReady));
    }

    #[test]
    fn roundtrip_boot_timing() {
        let msg = GuestToHost::BootTiming {
            stages: vec![
                BootStage { name: "squashfs".into(), duration_ms: 50 },
                BootStage { name: "network".into(), duration_ms: 120 },
            ],
        };
        let frame = encode_guest_msg(&msg).unwrap();
        let decoded = decode_guest_msg(&frame[4..]).unwrap();
        match decoded {
            GuestToHost::BootTiming { stages } => {
                assert_eq!(stages.len(), 2);
                assert_eq!(stages[0], BootStage { name: "squashfs".into(), duration_ms: 50 });
                assert_eq!(stages[1], BootStage { name: "network".into(), duration_ms: 120 });
            }
            other => panic!("expected BootTiming, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_boot_timing_empty() {
        let msg = GuestToHost::BootTiming { stages: vec![] };
        let frame = encode_guest_msg(&msg).unwrap();
        let decoded = decode_guest_msg(&frame[4..]).unwrap();
        match decoded {
            GuestToHost::BootTiming { stages } => assert!(stages.is_empty()),
            other => panic!("expected BootTiming, got {other:?}"),
        }
    }

    #[test]
    fn boot_timing_fails_as_host_msg() {
        let msg = GuestToHost::BootTiming {
            stages: vec![BootStage { name: "test".into(), duration_ms: 1 }],
        };
        let frame = encode_guest_msg(&msg).unwrap();
        assert!(decode_host_msg(&frame[4..]).is_err());
    }

    #[test]
    fn roundtrip_exec_done() {
        let msg = GuestToHost::ExecDone {
            id: 99,
            exit_code: 127,
        };
        let frame = encode_guest_msg(&msg).unwrap();
        let decoded = decode_guest_msg(&frame[4..]).unwrap();
        match decoded {
            GuestToHost::ExecDone { id, exit_code } => {
                assert_eq!(id, 99);
                assert_eq!(exit_code, 127);
            }
            other => panic!("expected ExecDone, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_pong() {
        let msg = GuestToHost::Pong;
        let frame = encode_guest_msg(&msg).unwrap();
        let decoded = decode_guest_msg(&frame[4..]).unwrap();
        assert!(matches!(decoded, GuestToHost::Pong));
    }

    #[test]
    fn roundtrip_file_created() {
        let msg = GuestToHost::FileCreated {
            path: "/workspace/new.txt".into(),
            size: 1234,
        };
        let frame = encode_guest_msg(&msg).unwrap();
        let decoded = decode_guest_msg(&frame[4..]).unwrap();
        match decoded {
            GuestToHost::FileCreated { path, size } => {
                assert_eq!(path, "/workspace/new.txt");
                assert_eq!(size, 1234);
            }
            other => panic!("expected FileCreated, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_file_modified() {
        let msg = GuestToHost::FileModified {
            path: "/workspace/edit.txt".into(),
            size: 5678,
        };
        let frame = encode_guest_msg(&msg).unwrap();
        let decoded = decode_guest_msg(&frame[4..]).unwrap();
        match decoded {
            GuestToHost::FileModified { path, size } => {
                assert_eq!(path, "/workspace/edit.txt");
                assert_eq!(size, 5678);
            }
            other => panic!("expected FileModified, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_file_deleted() {
        let msg = GuestToHost::FileDeleted {
            path: "/workspace/gone.txt".into(),
        };
        let frame = encode_guest_msg(&msg).unwrap();
        let decoded = decode_guest_msg(&frame[4..]).unwrap();
        match decoded {
            GuestToHost::FileDeleted { path } => assert_eq!(path, "/workspace/gone.txt"),
            other => panic!("expected FileDeleted, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_file_content() {
        let msg = GuestToHost::FileContent {
            id: 7,
            path: "/workspace/out.log".into(),
            data: b"log contents here".to_vec(),
        };
        let frame = encode_guest_msg(&msg).unwrap();
        let decoded = decode_guest_msg(&frame[4..]).unwrap();
        match decoded {
            GuestToHost::FileContent { id, path, data } => {
                assert_eq!(id, 7);
                assert_eq!(path, "/workspace/out.log");
                assert_eq!(data, b"log contents here");
            }
            other => panic!("expected FileContent, got {other:?}"),
        }
    }

    // -------------------------------------------------------------------
    // Frame format
    // -------------------------------------------------------------------

    #[test]
    fn frame_length_prefix_is_correct() {
        let msg = HostToGuest::Ping;
        let frame = encode_host_msg(&msg).unwrap();
        let len = u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize;
        assert_eq!(len, frame.len() - 4);
    }

    #[test]
    fn frame_length_prefix_is_big_endian() {
        let msg = HostToGuest::Ping;
        let frame = encode_host_msg(&msg).unwrap();
        let payload_len = frame.len() - 4;
        let expected = (payload_len as u32).to_be_bytes();
        assert_eq!(&frame[..4], &expected);
    }

    #[test]
    fn rmp_encoding_is_deterministic() {
        let msg = HostToGuest::Resize {
            cols: 80,
            rows: 24,
        };
        let a = encode_host_msg(&msg).unwrap();
        let b = encode_host_msg(&msg).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn different_messages_produce_different_bytes() {
        let ping = encode_host_msg(&HostToGuest::Ping).unwrap();
        let pong = encode_guest_msg(&GuestToHost::Pong).unwrap();
        assert_ne!(ping, pong);
    }

    #[test]
    fn rmp_payload_is_compact() {
        let frame = encode_host_msg(&HostToGuest::Ping).unwrap();
        let payload_len = frame.len() - 4;
        assert!(
            payload_len < 50,
            "Ping payload is {payload_len} bytes, expected < 50"
        );
    }

    // -------------------------------------------------------------------
    // Cross-type decode must fail (disjoint type safety)
    // -------------------------------------------------------------------

    #[test]
    fn guest_msg_fails_to_decode_as_host() {
        let msg = GuestToHost::Pong;
        let frame = encode_guest_msg(&msg).unwrap();
        let result = decode_host_msg(&frame[4..]);
        // Pong only exists in GuestToHost, not HostToGuest, so this must fail.
        assert!(result.is_err(), "decoding GuestToHost::Pong as HostToGuest should fail");
    }

    #[test]
    fn host_msg_fails_to_decode_as_guest() {
        let msg = HostToGuest::Ping;
        let frame = encode_host_msg(&msg).unwrap();
        let result = decode_guest_msg(&frame[4..]);
        // Ping only exists in HostToGuest, not GuestToHost, so this must fail.
        assert!(result.is_err(), "decoding HostToGuest::Ping as GuestToHost should fail");
    }

    #[test]
    fn boot_config_fails_as_guest_msg() {
        let msg = HostToGuest::BootConfig { epoch_secs: 1000 };
        let frame = encode_host_msg(&msg).unwrap();
        let result = decode_guest_msg(&frame[4..]);
        assert!(result.is_err());
    }

    #[test]
    fn boot_ready_fails_as_host_msg() {
        let msg = GuestToHost::BootReady;
        let frame = encode_guest_msg(&msg).unwrap();
        let result = decode_host_msg(&frame[4..]);
        assert!(result.is_err());
    }

    // -------------------------------------------------------------------
    // Decode error handling
    // -------------------------------------------------------------------

    #[test]
    fn decode_empty_payload_fails_host() {
        assert!(decode_host_msg(&[]).is_err());
    }

    #[test]
    fn decode_empty_payload_fails_guest() {
        assert!(decode_guest_msg(&[]).is_err());
    }

    #[test]
    fn decode_garbage_bytes_fails() {
        let garbage = [0xFF, 0xFE, 0xFD, 0xFC, 0xFB];
        assert!(decode_host_msg(&garbage).is_err());
        assert!(decode_guest_msg(&garbage).is_err());
    }

    #[test]
    fn decode_truncated_payload_fails() {
        let msg = GuestToHost::Ready {
            version: "1.0.0".into(),
        };
        let frame = encode_guest_msg(&msg).unwrap();
        let half = &frame[4..4 + (frame.len() - 4) / 2];
        assert!(decode_guest_msg(half).is_err());
    }

    // -------------------------------------------------------------------
    // SetEnv / BootConfigDone size validation
    // -------------------------------------------------------------------

    #[test]
    fn boot_config_done_is_compact() {
        let frame = encode_host_msg(&HostToGuest::BootConfigDone).unwrap();
        let payload_len = frame.len() - 4;
        assert!(
            payload_len < 50,
            "BootConfigDone payload is {payload_len} bytes, expected < 50"
        );
    }

    // -------------------------------------------------------------------
    // All variants fit within MAX_FRAME_SIZE
    // -------------------------------------------------------------------

    #[test]
    fn all_host_variants_fit() {
        let messages = vec![
            HostToGuest::BootConfig {
                epoch_secs: u64::MAX,
            },
            HostToGuest::SetEnv {
                key: "K".into(),
                value: "V".into(),
            },
            HostToGuest::BootConfigDone,
            HostToGuest::Resize {
                cols: u16::MAX,
                rows: u16::MAX,
            },
            HostToGuest::Exec {
                id: u64::MAX,
                command: "echo hello".into(),
            },
            HostToGuest::Ping,
            HostToGuest::Heartbeat { epoch_secs: u64::MAX },
            HostToGuest::FileWrite {
                path: "/test".into(),
                data: vec![0; 10],
                mode: 0o644,
            },
            HostToGuest::FileRead {
                id: 1,
                path: "/test".into(),
            },
            HostToGuest::FileDelete {
                path: "/test".into(),
            },
            HostToGuest::Shutdown,
        ];
        for msg in messages {
            let frame = encode_host_msg(&msg).unwrap();
            let payload_len = frame.len() - 4;
            assert!(
                payload_len <= MAX_FRAME_SIZE as usize,
                "{msg:?} payload is {payload_len} bytes, exceeds max {MAX_FRAME_SIZE}"
            );
        }
    }

    #[test]
    fn all_guest_variants_fit() {
        let messages = vec![
            GuestToHost::Ready {
                version: "99.99.99".into(),
            },
            GuestToHost::BootReady,
            GuestToHost::BootTiming {
                stages: vec![
                    BootStage { name: "squashfs".into(), duration_ms: 50 },
                    BootStage { name: "network".into(), duration_ms: 120 },
                ],
            },
            GuestToHost::ExecDone {
                id: u64::MAX,
                exit_code: i32::MIN,
            },
            GuestToHost::Pong,
            GuestToHost::FileCreated {
                path: "/test".into(),
                size: u64::MAX,
            },
            GuestToHost::FileModified {
                path: "/test".into(),
                size: u64::MAX,
            },
            GuestToHost::FileDeleted {
                path: "/test".into(),
            },
            GuestToHost::FileContent {
                id: 1,
                path: "/test".into(),
                data: vec![0; 10],
            },
        ];
        for msg in messages {
            let frame = encode_guest_msg(&msg).unwrap();
            let payload_len = frame.len() - 4;
            assert!(
                payload_len <= MAX_FRAME_SIZE as usize,
                "{msg:?} payload is {payload_len} bytes, exceeds max {MAX_FRAME_SIZE}"
            );
        }
    }

    // -------------------------------------------------------------------
    // Constants
    // -------------------------------------------------------------------

    #[test]
    fn max_frame_size_is_256kb() {
        assert_eq!(max_frame_size(), 262_144);
    }

    // -------------------------------------------------------------------
    // Edge cases
    // -------------------------------------------------------------------

    #[test]
    fn exec_done_negative_exit_code() {
        let msg = GuestToHost::ExecDone {
            id: 1,
            exit_code: -1,
        };
        let frame = encode_guest_msg(&msg).unwrap();
        let decoded = decode_guest_msg(&frame[4..]).unwrap();
        match decoded {
            GuestToHost::ExecDone { id, exit_code } => {
                assert_eq!(id, 1);
                assert_eq!(exit_code, -1);
            }
            other => panic!("expected ExecDone, got {other:?}"),
        }
    }

    #[test]
    fn exec_max_id() {
        let msg = HostToGuest::Exec {
            id: u64::MAX,
            command: "x".into(),
        };
        let frame = encode_host_msg(&msg).unwrap();
        let decoded = decode_host_msg(&frame[4..]).unwrap();
        match decoded {
            HostToGuest::Exec { id, .. } => assert_eq!(id, u64::MAX),
            other => panic!("expected Exec, got {other:?}"),
        }
    }

    #[test]
    fn ready_empty_version() {
        let msg = GuestToHost::Ready {
            version: String::new(),
        };
        let frame = encode_guest_msg(&msg).unwrap();
        let decoded = decode_guest_msg(&frame[4..]).unwrap();
        match decoded {
            GuestToHost::Ready { version } => assert_eq!(version, ""),
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    #[test]
    fn boot_config_zero_epoch() {
        let msg = HostToGuest::BootConfig { epoch_secs: 0 };
        let frame = encode_host_msg(&msg).unwrap();
        let decoded = decode_host_msg(&frame[4..]).unwrap();
        match decoded {
            HostToGuest::BootConfig { epoch_secs } => {
                assert_eq!(epoch_secs, 0);
            }
            other => panic!("expected BootConfig, got {other:?}"),
        }
    }

    #[test]
    fn large_file_write_fits_in_frame() {
        // A 200KB file should fit in the 256KB frame.
        let msg = HostToGuest::FileWrite {
            path: "/workspace/ca-bundle.crt".into(),
            data: vec![0x41; 200_000],
            mode: 0o644,
        };
        let frame = encode_host_msg(&msg).unwrap();
        let payload_len = frame.len() - 4;
        assert!(
            payload_len <= MAX_FRAME_SIZE as usize,
            "FileWrite payload is {payload_len} bytes, exceeds max {MAX_FRAME_SIZE}"
        );
    }

    // -------------------------------------------------------------------
    // Boot handshake validation: env key
    // -------------------------------------------------------------------

    #[test]
    fn validate_env_key_accepts_normal_keys() {
        assert!(validate_env_key("HOME").is_ok());
        assert!(validate_env_key("PATH").is_ok());
        assert!(validate_env_key("ANTHROPIC_API_KEY").is_ok());
        assert!(validate_env_key("MY_VAR_123").is_ok());
        assert!(validate_env_key("a").is_ok());
    }

    #[test]
    fn validate_env_key_rejects_empty() {
        assert!(validate_env_key("").is_err());
    }

    #[test]
    fn validate_env_key_rejects_equals() {
        assert!(validate_env_key("FOO=BAR").is_err());
        assert!(validate_env_key("=").is_err());
        assert!(validate_env_key("KEY=").is_err());
    }

    #[test]
    fn validate_env_key_rejects_nul() {
        assert!(validate_env_key("FOO\0BAR").is_err());
        assert!(validate_env_key("\0").is_err());
    }

    #[test]
    fn validate_env_key_rejects_oversized() {
        let long_key = "X".repeat(MAX_ENV_KEY_LEN + 1);
        assert!(validate_env_key(&long_key).is_err());
        // Exactly at limit should pass.
        let ok_key = "X".repeat(MAX_ENV_KEY_LEN);
        assert!(validate_env_key(&ok_key).is_ok());
    }

    #[test]
    fn validate_env_key_rejects_every_blocked_var() {
        for &var in BLOCKED_ENV_VARS {
            assert!(
                validate_env_key(var).is_err(),
                "should reject blocked var: {var}"
            );
        }
    }

    #[test]
    fn validate_env_key_rejects_ld_prefix_vars() {
        // LD_ prefix catch-all blocks unknown linker vars.
        assert!(validate_env_key("LD_TRACE_LOADED_OBJECTS").is_err());
        assert!(validate_env_key("LD_WHATEVER").is_err());
    }

    #[test]
    fn validate_env_key_rejects_bash_func_export() {
        assert!(validate_env_key("BASH_FUNC_myfunc%%").is_err());
        assert!(validate_env_key("BASH_FUNC_evil").is_err());
    }

    #[test]
    fn validate_env_key_case_sensitive() {
        // Linux env vars are case-sensitive. Lowercase variants are harmless.
        assert!(validate_env_key("ld_preload").is_ok());
        assert!(validate_env_key("Ld_Preload").is_ok());
        assert!(validate_env_key("ifs").is_ok());
        assert!(validate_env_key("bash_env").is_ok());
    }

    // -------------------------------------------------------------------
    // Boot handshake validation: env value
    // -------------------------------------------------------------------

    #[test]
    fn validate_env_value_accepts_normal() {
        assert!(validate_env_value("hello world").is_ok());
        assert!(validate_env_value("").is_ok()); // empty value is valid
        assert!(validate_env_value("/usr/bin:/usr/local/bin").is_ok());
        assert!(validate_env_value("sk-test-abc123").is_ok());
    }

    #[test]
    fn validate_env_value_rejects_nul() {
        assert!(validate_env_value("foo\0bar").is_err());
        assert!(validate_env_value("\0").is_err());
    }

    #[test]
    fn validate_env_value_rejects_oversized() {
        let long_val = "X".repeat(MAX_ENV_VALUE_LEN + 1);
        assert!(validate_env_value(&long_val).is_err());
        // Exactly at limit should pass.
        let ok_val = "X".repeat(MAX_ENV_VALUE_LEN);
        assert!(validate_env_value(&ok_val).is_ok());
    }

    // -------------------------------------------------------------------
    // Boot handshake validation: file path
    // -------------------------------------------------------------------

    #[test]
    fn validate_file_path_accepts_normal() {
        assert!(validate_file_path("/workspace/test.txt").is_ok());
        assert!(validate_file_path("/etc/ssl/certs/ca-certificates.crt").is_ok());
        assert!(validate_file_path("/root/.bashrc").is_ok());
    }

    #[test]
    fn validate_file_path_rejects_empty() {
        assert!(validate_file_path("").is_err());
    }

    #[test]
    fn validate_file_path_rejects_nul() {
        assert!(validate_file_path("/workspace/\0evil").is_err());
    }

    #[test]
    fn validate_file_path_rejects_traversal() {
        assert!(validate_file_path("/workspace/../etc/passwd").is_err());
        assert!(validate_file_path("../escape").is_err());
        assert!(validate_file_path("/workspace/..").is_err());
        assert!(validate_file_path("..").is_err());
    }

    // -------------------------------------------------------------------
    // is_blocked_env_var
    // -------------------------------------------------------------------

    #[test]
    fn is_blocked_catches_all_listed_vars() {
        assert!(is_blocked_env_var("LD_PRELOAD"));
        assert!(is_blocked_env_var("LD_LIBRARY_PATH"));
        assert!(is_blocked_env_var("LD_AUDIT"));
        assert!(is_blocked_env_var("IFS"));
        assert!(is_blocked_env_var("BASH_ENV"));
        assert!(is_blocked_env_var("ENV"));
        assert!(is_blocked_env_var("CDPATH"));
        assert!(is_blocked_env_var("GLOBIGNORE"));
        assert!(is_blocked_env_var("SHELLOPTS"));
        assert!(is_blocked_env_var("BASHOPTS"));
        assert!(is_blocked_env_var("PROMPT_COMMAND"));
        assert!(is_blocked_env_var("PS4"));
    }

    #[test]
    fn is_blocked_allows_safe_vars() {
        assert!(!is_blocked_env_var("HOME"));
        assert!(!is_blocked_env_var("PATH"));
        assert!(!is_blocked_env_var("TERM"));
        assert!(!is_blocked_env_var("EDITOR"));
        assert!(!is_blocked_env_var("ANTHROPIC_API_KEY"));
    }

    #[test]
    fn is_blocked_case_sensitive() {
        assert!(!is_blocked_env_var("ld_preload"));
        assert!(!is_blocked_env_var("Ld_Preload"));
        assert!(!is_blocked_env_var("ifs"));
    }

    // -------------------------------------------------------------------
    // Constants
    // -------------------------------------------------------------------

    #[test]
    fn boot_cap_constants() {
        assert_eq!(MAX_BOOT_ENV_VARS, 128);
        assert_eq!(MAX_BOOT_FILES, 64);
        assert_eq!(MAX_BOOT_FILE_BYTES, 10_485_760);
        assert_eq!(MAX_ENV_KEY_LEN, 256);
        assert_eq!(MAX_ENV_VALUE_LEN, 131_072);
    }

    // -------------------------------------------------------------------
    // File event edge cases
    // -------------------------------------------------------------------

    #[test]
    fn roundtrip_file_created_zero_size() {
        let msg = GuestToHost::FileCreated {
            path: "empty.txt".into(),
            size: 0,
        };
        let frame = encode_guest_msg(&msg).unwrap();
        let decoded = decode_guest_msg(&frame[4..]).unwrap();
        match decoded {
            GuestToHost::FileCreated { path, size } => {
                assert_eq!(path, "empty.txt");
                assert_eq!(size, 0);
            }
            other => panic!("expected FileCreated, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_file_created_max_size() {
        let msg = GuestToHost::FileCreated {
            path: "huge.bin".into(),
            size: u64::MAX,
        };
        let frame = encode_guest_msg(&msg).unwrap();
        let decoded = decode_guest_msg(&frame[4..]).unwrap();
        match decoded {
            GuestToHost::FileCreated { path, size } => {
                assert_eq!(path, "huge.bin");
                assert_eq!(size, u64::MAX);
            }
            other => panic!("expected FileCreated, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_file_created_empty_path() {
        let msg = GuestToHost::FileCreated {
            path: "".into(),
            size: 42,
        };
        let frame = encode_guest_msg(&msg).unwrap();
        let decoded = decode_guest_msg(&frame[4..]).unwrap();
        match decoded {
            GuestToHost::FileCreated { path, size } => {
                assert_eq!(path, "");
                assert_eq!(size, 42);
            }
            other => panic!("expected FileCreated, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_file_deleted_empty_path() {
        let msg = GuestToHost::FileDeleted {
            path: "".into(),
        };
        let frame = encode_guest_msg(&msg).unwrap();
        let decoded = decode_guest_msg(&frame[4..]).unwrap();
        match decoded {
            GuestToHost::FileDeleted { path } => assert_eq!(path, ""),
            other => panic!("expected FileDeleted, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_file_modified_unicode_path() {
        let unicode_path = "project/\u{1F4C4}\u{4E2D}\u{6587}/caf\u{00E9}.rs";
        let msg = GuestToHost::FileModified {
            path: unicode_path.into(),
            size: 100,
        };
        let frame = encode_guest_msg(&msg).unwrap();
        let decoded = decode_guest_msg(&frame[4..]).unwrap();
        match decoded {
            GuestToHost::FileModified { path, size } => {
                assert_eq!(path, unicode_path);
                assert_eq!(size, 100);
            }
            other => panic!("expected FileModified, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_file_created_long_path() {
        let long_path = "a/".repeat(5000) + "file.txt";
        let msg = GuestToHost::FileCreated {
            path: long_path.clone(),
            size: 1,
        };
        let frame = encode_guest_msg(&msg).unwrap();
        let decoded = decode_guest_msg(&frame[4..]).unwrap();
        match decoded {
            GuestToHost::FileCreated { path, size } => {
                assert_eq!(path, long_path);
                assert_eq!(size, 1);
            }
            other => panic!("expected FileCreated, got {other:?}"),
        }
    }

    #[test]
    fn cross_decode_host_format() {
        // Encode with rmp_serde directly (simulating remote), decode with helper.
        let msg = HostToGuest::Resize {
            cols: 132,
            rows: 43,
        };
        let raw = rmp_serde::to_vec_named(&msg).unwrap();
        let decoded = decode_host_msg(&raw).unwrap();
        match decoded {
            HostToGuest::Resize { cols, rows } => {
                assert_eq!(cols, 132);
                assert_eq!(rows, 43);
            }
            other => panic!("expected Resize, got {other:?}"),
        }
    }

}
