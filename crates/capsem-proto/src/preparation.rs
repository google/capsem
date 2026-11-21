//! Private, bounded startup-wrapper status. Script output has a different FD.

use serde::{Deserialize, Serialize};
use std::io::{self, Cursor};

/// One u32-BE framed named MessagePack status. No output or script bytes.
pub const MAX_STATUS_BYTES: u32 = 256;
/// The startup wrapper inherits these descriptors from its guest controller.
/// Both are closed across script and entrypoint exec; script stdout/stderr
/// receive duplicates of OUTPUT_FD only.
pub const OUTPUT_FD: i32 = 3;
pub const STATUS_FD: i32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PreparationStatus {
    Preparing,
    /// Preparation succeeded. The controller waits for status EOF from exec
    /// before reporting running; an exec failure may still follow this record.
    Entrypoint,
    Failed {
        failure: PreparationFailure,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PreparationFailure {
    InvalidInput,
    ShellUnavailable,
    Spawn,
    Wait,
    Exit { code: i32 },
    Signal { signal: i32 },
    TimedOut,
    EntrypointExec,
}

fn validate(status: &PreparationStatus) -> io::Result<()> {
    let valid = match status {
        PreparationStatus::Failed {
            failure: PreparationFailure::Exit { code },
        } => (1..=255).contains(code),
        PreparationStatus::Failed {
            failure: PreparationFailure::Signal { signal },
        } => (1..=64).contains(signal),
        _ => true,
    };
    if !valid {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid preparation result"));
    }
    Ok(())
}

pub fn encode(status: &PreparationStatus) -> io::Result<Vec<u8>> {
    validate(status)?;
    crate::framing::encode_frame(status, MAX_STATUS_BYTES)
}

/// Refuse truncated, concatenated or oversized status records. The private
/// pipe is descriptive state only; it never carries credentials or policy.
pub fn decode(payload: &[u8]) -> io::Result<PreparationStatus> {
    if payload.is_empty() || payload.len() > MAX_STATUS_BYTES as usize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid preparation status length",
        ));
    }
    let mut input = Cursor::new(payload);
    let status = {
        let mut decoder = rmp_serde::Deserializer::new(&mut input);
        decoder.set_max_depth(8);
        PreparationStatus::deserialize(&mut decoder)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid preparation status"))?
    };
    if input.position() as usize != payload.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "trailing preparation status bytes",
        ));
    }
    validate(&status)?;
    Ok(status)
}

#[cfg(test)]
mod tests;
