//! Shared typed terminal/exec/container stream messages. Transport adapters
//! own authentication and framing, never a second message vocabulary.
use std::borrow::Cow;
use std::io::{Read, Write};

use crate::terminal::TerminalState;
use serde::{Deserialize, Serialize};

/// A MessagePack record, excluding a byte-stream length prefix.
pub const MAX_FRAME_BYTES: usize = 512 * 1024;
/// Data shares the existing exec lane budget.
pub const MAX_DATA_BYTES: usize = crate::exec_stream::MAX_EXEC_DATA_BYTES;

/// What a stream attaches to.
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StreamKind {
    /// The VM's interactive terminal, with its reconstructed current display.
    Terminal,
    /// One command, streaming its output until it exits.
    Exec,
    /// The VM's container workload, attached until it exits.
    Container,
}

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamControl {
    /// First control message of every stream. `command` is required for
    /// `exec` and refused for the others.
    Start {
        kind: StreamKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command: Option<String>,
    },
    /// Terminal window size; both dimensions must be non-zero.
    Resize {
        #[cfg_attr(feature = "schema", schema(schema_with = crate::terminal::schema::cols))]
        cols: u16,
        #[cfg_attr(feature = "schema", schema(schema_with = crate::terminal::schema::rows))]
        rows: u16,
    },
    /// This view explicitly takes input and resize authority.
    Focus {
        #[cfg_attr(feature = "schema", schema(schema_with = crate::terminal::schema::cols))]
        cols: u16,
        #[cfg_attr(feature = "schema", schema(schema_with = crate::terminal::schema::rows))]
        rows: u16,
    },
    /// No more stdin will follow.
    CloseStdin,
}

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamStatus {
    /// The stream is attached; data may follow.
    Started,
    /// Authoritative terminal metadata, followed by normalized display bytes.
    Terminal { state: Box<TerminalState> },
    /// The command or workload ended. `truncated` means output was dropped.
    Exit { code: i32, truncated: bool },
    /// The stream ended without an exit status.
    Error { message: String },
}

/// A named record with real MessagePack binary values. Decoding borrows data
/// directly from the bounded incoming record; encoding metadata borrows the
/// existing typed value rather than cloning a terminal update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Message<'a> {
    Stdin(#[serde(borrow, with = "serde_bytes")] &'a [u8]),
    Stdout(#[serde(borrow, with = "serde_bytes")] &'a [u8]),
    Stderr(#[serde(borrow, with = "serde_bytes")] &'a [u8]),
    Control(Cow<'a, StreamControl>),
    Status(Cow<'a, StreamStatus>),
}

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("stream frame exceeds its {MAX_FRAME_BYTES}-byte budget")]
    TooLarge,
    #[error("stream message has the wrong direction")]
    WrongDirection,
    #[error("invalid stream message: {0}")]
    Invalid(String),
    #[error("stream I/O: {0}")]
    Io(#[from] std::io::Error),
}

impl StreamControl {
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::Resize { cols, rows } | Self::Focus { cols, rows } => crate::terminal::validate_size(*cols, *rows),
            Self::Start {
                kind: StreamKind::Exec,
                command,
            } if command.as_deref().is_none_or(str::is_empty) => Err("exec needs a command"),
            Self::Start {
                kind: StreamKind::Terminal | StreamKind::Container,
                command: Some(_),
            } => Err("only exec takes a command"),
            _ => Ok(()),
        }
    }
}

impl Message<'_> {
    fn validate(&self) -> Result<(), FrameError> {
        let result = match self {
            Self::Stdin(data) | Self::Stdout(data) | Self::Stderr(data) if data.len() > MAX_DATA_BYTES => {
                Err("stream data exceeds its budget")
            }
            Self::Control(control) => control.validate(),
            Self::Status(status) => match status.as_ref() {
                StreamStatus::Terminal { state } => state.validate(),
                _ => Ok(()),
            },
            _ => Ok(()),
        };
        result.map_err(|error| FrameError::Invalid(error.into()))
    }
}

pub fn encode(message: &Message<'_>) -> Result<Vec<u8>, FrameError> {
    message.validate()?;
    let payload = rmp_serde::to_vec_named(message).map_err(|error| FrameError::Invalid(error.to_string()))?;
    if payload.len() > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge);
    }
    Ok(payload)
}

fn decode(payload: &[u8]) -> Result<Message<'_>, FrameError> {
    if payload.len() > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge);
    }
    // serde_bytes accepts MessagePack strings too, including invalid UTF-8.
    // Use the underlying codec's header readers to require the named-map and
    // binary-field contract before Serde borrows the payload.
    let mut fields = payload;
    let invalid = |error: rmp::decode::ValueReadError| FrameError::Invalid(error.to_string());
    if rmp::decode::read_map_len(&mut fields).map_err(invalid)? != 1 {
        return Err(FrameError::Invalid("stream record must have one named field".into()));
    }
    let mut key = [0; 16];
    let lane = rmp::decode::read_str(&mut fields, &mut key).map_err(|error| FrameError::Invalid(error.to_string()))?;
    if matches!(lane, "stdin" | "stdout" | "stderr") {
        let length = rmp::decode::read_bin_len(&mut fields).map_err(invalid)? as usize;
        if length > MAX_DATA_BYTES || fields.len() != length {
            return Err(FrameError::Invalid("invalid stream binary payload length".into()));
        }
    }
    let mut decoder = rmp_serde::Deserializer::from_read_ref(payload);
    decoder.set_max_depth(64);
    let message = Message::deserialize(&mut decoder).map_err(|error| FrameError::Invalid(error.to_string()))?;
    // rmp-serde's slice helper allows trailing data. A missing next marker is
    // the only accepted end condition. Decode a scalar, not IgnoredAny: the
    // latter could consume a truncated nested value and also report EOF.
    match u8::deserialize(&mut decoder) {
        Err(rmp_serde::decode::Error::InvalidMarkerRead(error))
            if error.kind() == std::io::ErrorKind::UnexpectedEof => {}
        _ => return Err(FrameError::Invalid("trailing bytes after stream record".into())),
    }
    message.validate()?;
    Ok(message)
}

pub fn decode_client(payload: &[u8]) -> Result<Message<'_>, FrameError> {
    let message = decode(payload)?;
    match message {
        Message::Stdin(_) | Message::Control(_) => Ok(message),
        _ => Err(FrameError::WrongDirection),
    }
}

pub fn decode_server(payload: &[u8]) -> Result<Message<'_>, FrameError> {
    let message = decode(payload)?;
    match message {
        Message::Stdout(_) | Message::Stderr(_) | Message::Status(_) => Ok(message),
        _ => Err(FrameError::WrongDirection),
    }
}

/// Byte-stream transports add the same bounded u32-BE header as other Capsem
/// IPC. WebSocket transports already delimit records and use `encode`.
pub fn framed(message: &Message<'_>) -> Result<Vec<u8>, FrameError> {
    let payload = encode(message)?;
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.write_all(&(payload.len() as u32).to_be_bytes())?;
    frame.write_all(&payload)?;
    Ok(frame)
}

pub fn read_payload(reader: &mut impl Read) -> Result<Vec<u8>, FrameError> {
    let mut header = [0; 4];
    reader.read_exact(&mut header)?;
    let length = u32::from_be_bytes(header) as usize;
    if length == 0 || length > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge);
    }
    let mut payload = vec![0; length];
    reader.read_exact(&mut payload)?;
    Ok(payload)
}

#[cfg(test)]
mod tests;
