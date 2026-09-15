//! `capsem.stream.v1`: one WebSocket protocol for terminal I/O, streaming exec
//! and container attach.
//!
//! Every message is a binary WebSocket frame whose first byte names a channel.
//! Data channels carry raw bytes; the control and status channels carry one
//! typed JSON object. Each side decodes only the channels it may receive.

use serde::{Deserialize, Serialize};

/// WebSocket subprotocol a client must request.
pub const STREAM_SUBPROTOCOL: &str = "capsem.stream.v1";

/// Largest frame either side accepts, channel byte included.
pub const MAX_STREAM_FRAME_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum StreamChannel {
    /// Client to server: bytes for the stream's stdin or terminal.
    Stdin = 0,
    /// Server to client: output (merged stdout and stderr today).
    Stdout = 1,
    /// Server to client: reserved for separated stderr.
    Stderr = 2,
    /// Client to server: [`StreamControl`].
    Control = 3,
    /// Server to client: [`StreamStatus`].
    Status = 4,
}

/// What a stream attaches to.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StreamKind {
    /// The VM's interactive terminal, with its replayed recent output.
    Terminal,
    /// One command, streaming its output until it exits.
    Exec,
    /// The VM's container workload, attached until it exits.
    Container,
}

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
    Resize { cols: u16, rows: u16 },
    /// No more stdin will follow.
    CloseStdin,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamStatus {
    /// The stream is attached; data may follow.
    Started,
    /// The command or workload ended. `truncated` means output was dropped.
    Exit { code: i32, truncated: bool },
    /// The stream ended without an exit status.
    Error { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientFrame<'a> {
    Stdin(&'a [u8]),
    Control(StreamControl),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerFrame<'a> {
    Stdout(&'a [u8]),
    Stderr(&'a [u8]),
    Status(StreamStatus),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamFrameError {
    Empty,
    TooLarge(usize),
    UnknownChannel(u8),
    /// A channel the receiving side never accepts.
    WrongDirection(u8),
    /// A control or status payload that is not valid for the protocol.
    Control(String),
}

impl std::fmt::Display for StreamFrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "empty stream frame"),
            Self::TooLarge(size) => write!(f, "stream frame of {size} bytes exceeds {MAX_STREAM_FRAME_BYTES}"),
            Self::UnknownChannel(channel) => write!(f, "unknown stream channel {channel}"),
            Self::WrongDirection(channel) => write!(f, "stream channel {channel} is not accepted here"),
            Self::Control(reason) => write!(f, "invalid stream control: {reason}"),
        }
    }
}

impl std::error::Error for StreamFrameError {}

fn split(frame: &[u8]) -> Result<(u8, &[u8]), StreamFrameError> {
    if frame.len() > MAX_STREAM_FRAME_BYTES {
        return Err(StreamFrameError::TooLarge(frame.len()));
    }
    let (&channel, payload) = frame.split_first().ok_or(StreamFrameError::Empty)?;
    if channel > StreamChannel::Status as u8 {
        return Err(StreamFrameError::UnknownChannel(channel));
    }
    Ok((channel, payload))
}

pub fn decode_client_frame(frame: &[u8]) -> Result<ClientFrame<'_>, StreamFrameError> {
    let (channel, payload) = split(frame)?;
    match channel {
        c if c == StreamChannel::Stdin as u8 => Ok(ClientFrame::Stdin(payload)),
        c if c == StreamChannel::Control as u8 => {
            let control: StreamControl =
                serde_json::from_slice(payload).map_err(|e| StreamFrameError::Control(e.to_string()))?;
            validate_control(&control)?;
            Ok(ClientFrame::Control(control))
        }
        other => Err(StreamFrameError::WrongDirection(other)),
    }
}

pub fn decode_server_frame(frame: &[u8]) -> Result<ServerFrame<'_>, StreamFrameError> {
    let (channel, payload) = split(frame)?;
    match channel {
        c if c == StreamChannel::Stdout as u8 => Ok(ServerFrame::Stdout(payload)),
        c if c == StreamChannel::Stderr as u8 => Ok(ServerFrame::Stderr(payload)),
        c if c == StreamChannel::Status as u8 => serde_json::from_slice(payload)
            .map(ServerFrame::Status)
            .map_err(|e| StreamFrameError::Control(e.to_string())),
        other => Err(StreamFrameError::WrongDirection(other)),
    }
}

fn validate_control(control: &StreamControl) -> Result<(), StreamFrameError> {
    let invalid = |reason: &str| Err(StreamFrameError::Control(reason.to_owned()));
    match control {
        StreamControl::Resize { cols, rows } if *cols == 0 || *rows == 0 => invalid("resize needs non-zero dimensions"),
        StreamControl::Start {
            kind: StreamKind::Exec,
            command,
        } if command.as_deref().is_none_or(str::is_empty) => invalid("exec needs a command"),
        StreamControl::Start {
            kind: StreamKind::Terminal | StreamKind::Container,
            command: Some(_),
        } => invalid("only exec takes a command"),
        _ => Ok(()),
    }
}

/// A data frame on `channel`.
pub fn encode_data(channel: StreamChannel, data: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(data.len() + 1);
    frame.push(channel as u8);
    frame.extend_from_slice(data);
    frame
}

fn encode_json(channel: StreamChannel, value: &impl Serialize) -> Vec<u8> {
    let mut frame = vec![channel as u8];
    serde_json::to_writer(&mut frame, value).expect("stream control types serialize");
    frame
}

pub fn encode_control(control: &StreamControl) -> Vec<u8> {
    encode_json(StreamChannel::Control, control)
}

pub fn encode_status(status: &StreamStatus) -> Vec<u8> {
    encode_json(StreamChannel::Status, status)
}

#[cfg(test)]
mod tests;
