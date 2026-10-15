//! Fixed records carrying model-to-filesystem correlation hints.
//!
//! The confined proxy may name only a trace and a workspace-relative path.
//! The VM owner validates and stores that hint; the channel grants no read,
//! filesystem, network, or ledger authority back to the proxy.

use thiserror::Error;

pub const MAX_TRACE_HINT_ID_BYTES: usize = 64;
pub const MAX_TRACE_HINT_PATH_BYTES: usize = 4096;
pub const PROXY_TRACE_HINT_FRAME_SIZE: usize = 2 + 1 + 1 + 2 + MAX_TRACE_HINT_ID_BYTES + MAX_TRACE_HINT_PATH_BYTES;
/// Owner acknowledgement. The proxy awaits this before completing the model
/// response, so a following workspace write cannot outrun attribution state.
pub const PROXY_TRACE_HINT_ACK: [u8; 4] = *b"TH\x01A";

const MAGIC: [u8; 2] = *b"TH";
const VERSION: u8 = 1;
const TRACE_LENGTH_OFFSET: usize = 3;
const PATH_LENGTH_RANGE: std::ops::Range<usize> = 4..6;
const TRACE_RANGE: std::ops::Range<usize> = 6..6 + MAX_TRACE_HINT_ID_BYTES;
const PATH_RANGE: std::ops::Range<usize> = TRACE_RANGE.end..TRACE_RANGE.end + MAX_TRACE_HINT_PATH_BYTES;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProxyTraceHint {
    pub trace_id: String,
    pub relative_path: String,
}

pub fn encode_proxy_trace_hint(
    hint: &ProxyTraceHint,
) -> Result<[u8; PROXY_TRACE_HINT_FRAME_SIZE], ProxyTraceHintError> {
    validate_trace_id(&hint.trace_id)?;
    let path = hint.relative_path.as_bytes();
    if path.is_empty() || path.len() > MAX_TRACE_HINT_PATH_BYTES {
        return Err(ProxyTraceHintError::InvalidPath);
    }
    let mut frame = [0; PROXY_TRACE_HINT_FRAME_SIZE];
    frame[..2].copy_from_slice(&MAGIC);
    frame[2] = VERSION;
    frame[TRACE_LENGTH_OFFSET] = u8::try_from(hint.trace_id.len()).expect("trace length is bounded below u8::MAX");
    frame[PATH_LENGTH_RANGE].copy_from_slice(&(path.len() as u16).to_be_bytes());
    frame[TRACE_RANGE.start..TRACE_RANGE.start + hint.trace_id.len()].copy_from_slice(hint.trace_id.as_bytes());
    frame[PATH_RANGE.start..PATH_RANGE.start + path.len()].copy_from_slice(path);
    Ok(frame)
}

pub fn decode_proxy_trace_hint(
    frame: &[u8; PROXY_TRACE_HINT_FRAME_SIZE],
) -> Result<ProxyTraceHint, ProxyTraceHintError> {
    if frame[..2] != MAGIC {
        return Err(ProxyTraceHintError::Magic);
    }
    if frame[2] != VERSION {
        return Err(ProxyTraceHintError::Version(frame[2]));
    }
    let trace_len = usize::from(frame[TRACE_LENGTH_OFFSET]);
    let path_len = usize::from(u16::from_be_bytes(
        frame[PATH_LENGTH_RANGE].try_into().expect("fixed path length range"),
    ));
    if trace_len == 0 || trace_len > MAX_TRACE_HINT_ID_BYTES {
        return Err(ProxyTraceHintError::InvalidTraceId);
    }
    if path_len == 0 || path_len > MAX_TRACE_HINT_PATH_BYTES {
        return Err(ProxyTraceHintError::InvalidPath);
    }
    if frame[TRACE_RANGE.start + trace_len..TRACE_RANGE.end]
        .iter()
        .chain(frame[PATH_RANGE.start + path_len..PATH_RANGE.end].iter())
        .any(|byte| *byte != 0)
    {
        return Err(ProxyTraceHintError::ReservedBytes);
    }
    let trace_id = std::str::from_utf8(&frame[TRACE_RANGE.start..TRACE_RANGE.start + trace_len])
        .map_err(|_| ProxyTraceHintError::InvalidTraceId)?
        .to_string();
    validate_trace_id(&trace_id)?;
    let relative_path = std::str::from_utf8(&frame[PATH_RANGE.start..PATH_RANGE.start + path_len])
        .map_err(|_| ProxyTraceHintError::InvalidPath)?
        .to_string();
    Ok(ProxyTraceHint {
        trace_id,
        relative_path,
    })
}

fn validate_trace_id(trace_id: &str) -> Result<(), ProxyTraceHintError> {
    let lowercase_hex = |byte: u8| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte);
    let bytes = trace_id.as_bytes();
    let canonical = match bytes.len() {
        16 => bytes.iter().copied().all(lowercase_hex),
        36 => bytes.iter().copied().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                lowercase_hex(byte)
            }
        }),
        _ => false,
    } && bytes.iter().any(|byte| *byte != b'0' && *byte != b'-');
    if canonical {
        Ok(())
    } else {
        Err(ProxyTraceHintError::InvalidTraceId)
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ProxyTraceHintError {
    #[error("invalid proxy trace-hint magic")]
    Magic,
    #[error("unsupported proxy trace-hint version {0}")]
    Version(u8),
    #[error("trace id is not canonical or exceeds its bound")]
    InvalidTraceId,
    #[error("trace-hint path is empty, invalid UTF-8, or exceeds its bound")]
    InvalidPath,
    #[error("proxy trace-hint reserved bytes are nonzero")]
    ReservedBytes,
}

#[cfg(test)]
mod tests;
