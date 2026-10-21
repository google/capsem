//! `capsem.stream.v3`: one WebSocket protocol for terminal I/O, streaming exec
//! and container attach.
//!
//! Each binary WebSocket message is one shared, named MessagePack record.
//! Data uses binary fields; metadata uses the capsem-proto types. WebSocket
//! supplies the record boundary, so it needs no extra length prefix.

pub use capsem_proto::terminal::TerminalState;
pub use capsem_proto::TERMINAL_SUBSCRIBER_UPDATES as MAX_TERMINAL_VIEW_UPDATES;
pub use capsem_proto::{
    TERMINAL_DEFAULT_COLS as DEFAULT_TERMINAL_COLS, TERMINAL_DEFAULT_ROWS as DEFAULT_TERMINAL_ROWS,
};

/// WebSocket subprotocol a client must request.
pub const STREAM_SUBPROTOCOL: &str = "capsem.stream.v3";

/// Shared frame and binary-payload budgets, including envelope headroom.
pub use capsem_proto::stream::{MAX_DATA_BYTES as MAX_STREAM_DATA_BYTES, MAX_FRAME_BYTES as MAX_STREAM_FRAME_BYTES};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamChannel {
    /// Client to server: bytes for the stream's stdin or terminal.
    Stdin,
    /// Server to client: standard output.
    Stdout,
    /// Server to client: standard error.
    Stderr,
}

pub use capsem_proto::stream::{StreamControl, StreamKind, StreamStatus};

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

pub use capsem_proto::stream::FrameError as StreamFrameError;
use capsem_proto::stream::{self as wire, Message};
use std::borrow::Cow;

pub fn decode_client_frame(frame: &[u8]) -> Result<ClientFrame<'_>, StreamFrameError> {
    match wire::decode_client(frame)? {
        Message::Stdin(data) => Ok(ClientFrame::Stdin(data)),
        Message::Control(control) => Ok(ClientFrame::Control(control.into_owned())),
        _ => Err(StreamFrameError::WrongDirection),
    }
}

pub fn decode_server_frame(frame: &[u8]) -> Result<ServerFrame<'_>, StreamFrameError> {
    match wire::decode_server(frame)? {
        Message::Stdout(data) => Ok(ServerFrame::Stdout(data)),
        Message::Stderr(data) => Ok(ServerFrame::Stderr(data)),
        Message::Status(status) => Ok(ServerFrame::Status(status.into_owned())),
        _ => Err(StreamFrameError::WrongDirection),
    }
}

/// Callers split data at MAX_STREAM_DATA_BYTES before encoding.
pub fn encode_data(channel: StreamChannel, data: &[u8]) -> Result<Vec<u8>, StreamFrameError> {
    let message = match channel {
        StreamChannel::Stdin => Message::Stdin(data),
        StreamChannel::Stdout => Message::Stdout(data),
        StreamChannel::Stderr => Message::Stderr(data),
    };
    wire::encode(&message)
}

pub fn encode_control(control: &StreamControl) -> Result<Vec<u8>, StreamFrameError> {
    wire::encode(&Message::Control(Cow::Borrowed(control)))
}

pub fn encode_status(status: &StreamStatus) -> Result<Vec<u8>, StreamFrameError> {
    wire::encode(&Message::Status(Cow::Borrowed(status)))
}

/// The frames every other implementation of this protocol is held to,
/// exported as `sdk/specification/stream-v3.json`. Client frames are what this
/// module encodes; each server frame's expectation is this module's own
/// decoding of it, refusals included, so the fixture cannot disagree with Rust.
pub fn golden_frames() -> serde_json::Value {
    use serde_json::json;
    let controls = [
        StreamControl::Start {
            kind: StreamKind::Terminal,
            command: None,
        },
        StreamControl::Start {
            kind: StreamKind::Container,
            command: None,
        },
        StreamControl::Start {
            kind: StreamKind::Exec,
            command: Some("echo hi".into()),
        },
        StreamControl::Resize { cols: 120, rows: 40 },
        StreamControl::Focus { cols: 120, rows: 40 },
        StreamControl::CloseStdin,
    ];
    let mut client: Vec<_> = controls
        .iter()
        .map(|control| json!({ "control": control, "frame": encode_control(control).expect("fixture control") }))
        .collect();
    let stdin = [b'l', b's', 0, 255, b'\n'];
    client.push(json!({ "stdin": stdin, "frame": encode_data(StreamChannel::Stdin, &stdin).expect("fixture stdin") }));

    let statuses = [
        StreamStatus::Terminal {
            state: Box::new(TerminalState {
                revision: 7,
                reset: true,
                cols: 120,
                rows: 40,
                cursor: capsem_proto::terminal::TerminalCursor {
                    row: 3,
                    col: 7,
                    hidden: false,
                },
                modes: capsem_proto::terminal::TerminalModes::default(),
                title: "agent".into(),
                icon_name: "".into(),
                bell_count: 1,
                last_notification: Some(crate::TerminalNotification {
                    id: "notice-1".into(),
                    title: "Ready".into(),
                    body: "Review output".into(),
                }),
                events: Vec::new(),
            }),
        },
        StreamStatus::Started,
        StreamStatus::Exit {
            code: 0,
            truncated: false,
        },
        StreamStatus::Exit {
            code: 137,
            truncated: true,
        },
        StreamStatus::Error {
            message: "VM owner closed".into(),
        },
    ];
    let mut frames = vec![
        encode_data(StreamChannel::Stdout, b"hello\n").expect("fixture stdout"),
        encode_data(StreamChannel::Stderr, &[0, 255, 10]).expect("fixture stderr"),
    ];
    frames.extend(
        statuses
            .iter()
            .map(|status| encode_status(status).expect("fixture status")),
    );
    frames.extend([
        Vec::new(),
        vec![0xc1], // Reserved MessagePack marker.
        encode_data(StreamChannel::Stdin, b"ls").expect("fixture stdin"),
        encode_control(&StreamControl::CloseStdin).expect("fixture close"),
        vec![0x81, 0xa6, b's', b't', b'a', b't', b'u', b's', 0xc0],
    ]);
    let server: Vec<_> = frames
        .iter()
        .map(|frame| {
            let expect = match decode_server_frame(frame) {
                Ok(ServerFrame::Stdout(bytes) | ServerFrame::Stderr(bytes)) => {
                    json!({ "kind": "output", "bytes": bytes })
                }
                Ok(ServerFrame::Status(status)) => json!({ "kind": "status", "status": status }),
                Err(_) => json!({ "kind": "invalid" }),
            };
            json!({ "frame": frame, "expect": expect })
        })
        .collect();
    json!({
        "subprotocol": STREAM_SUBPROTOCOL,
        "max_frame_bytes": MAX_STREAM_FRAME_BYTES,
        "max_data_bytes": MAX_STREAM_DATA_BYTES,
        "client": client,
        "server": server,
        "oversized_frame_bytes": MAX_STREAM_FRAME_BYTES + 1,
    })
}

#[cfg(test)]
mod tests;
