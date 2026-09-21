//! Bounded bidirectional frames on the dedicated guest exec VSOCK.

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::io::{self, Read, Write};

/// Largest stdin or output chunk carried by one exec frame.
///
/// One byte below the stream frame ceiling: the service relays a data frame
/// with a one-byte channel prefix, so a chunk of exactly that ceiling became a
/// frame the CLI and TUI decoders refused as too large, ending the stream with
/// no exit status. `capsem_api::stream` asserts the relationship.
pub const MAX_EXEC_DATA_BYTES: usize = 256 * 1024 - 1;
/// Stdin frames (data or EOF) one streaming exec may have in flight between
/// the service and the guest. The VM owner queues at most this many and
/// reports each one it hands to the guest with
/// `ProcessToService::ExecInputConsumed`; the service sends a
/// frame only while it holds credit, so neither side ever waits on the other.
pub const EXEC_STDIN_WINDOW: usize = 16;
/// Largest encoded exec frame. The typed envelope stays well below this when
/// its data is at [`MAX_EXEC_DATA_BYTES`].
const MAX_EXEC_FRAME_BYTES: u32 = 512 * 1024;

/// Encoding used after the `ExecStarted` control frame on the dedicated exec
/// connection. Profiles released before typed streams sent one raw merged
/// stdout/stderr byte stream; current agents preserve the two output lanes.
#[derive(Serialize, Deserialize, Debug, Default, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecOutputProtocol {
    /// Compatibility format used by immutable profiles released before typed
    /// exec streams. The host reports the merged bytes as stdout.
    #[default]
    RawMerged,
    /// Length-prefixed MessagePack frames with explicit stdout/stderr lanes.
    FramedLanes,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum ExecInputFrame {
    Data(#[serde(with = "serde_bytes")] Vec<u8>),
    /// No more stdin follows. EOF is in-band because VSOCK half-close is not
    /// reliable on every supported hypervisor.
    StdinEof,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecOutputChannel {
    Stdout,
    Stderr,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ExecOutputFrame {
    pub channel: ExecOutputChannel,
    #[serde(with = "serde_bytes")]
    pub data: Vec<u8>,
}

pub fn write_exec_input(writer: &mut impl Write, frame: &ExecInputFrame) -> io::Result<()> {
    let data_len = match frame {
        ExecInputFrame::Data(data) => data.len(),
        ExecInputFrame::StdinEof => 0,
    };
    write_frame(writer, frame, data_len)
}

pub fn read_exec_input(reader: &mut impl Read) -> io::Result<ExecInputFrame> {
    let frame: ExecInputFrame = read_frame(reader)?;
    if matches!(&frame, ExecInputFrame::Data(data) if data.len() > MAX_EXEC_DATA_BYTES) {
        return Err(too_large("exec input data"));
    }
    Ok(frame)
}

pub fn write_exec_output(writer: &mut impl Write, frame: &ExecOutputFrame) -> io::Result<()> {
    write_exec_output_data(writer, frame.channel, &frame.data)
}

/// Frame bytes the caller just read, without first copying them into an owned
/// frame. Encodes exactly what the equivalent `ExecOutputFrame` does.
pub fn write_exec_output_data(writer: &mut impl Write, channel: ExecOutputChannel, data: &[u8]) -> io::Result<()> {
    #[derive(Serialize)]
    struct Borrowed<'a> {
        channel: ExecOutputChannel,
        #[serde(with = "serde_bytes")]
        data: &'a [u8],
    }
    write_frame(writer, &Borrowed { channel, data }, data.len())
}

pub fn read_exec_output(reader: &mut impl Read) -> io::Result<ExecOutputFrame> {
    let frame: ExecOutputFrame = read_frame(reader)?;
    if frame.data.len() > MAX_EXEC_DATA_BYTES {
        return Err(too_large("exec output data"));
    }
    Ok(frame)
}

fn write_frame(writer: &mut impl Write, frame: &impl Serialize, data_len: usize) -> io::Result<()> {
    if data_len > MAX_EXEC_DATA_BYTES {
        return Err(too_large("exec frame data"));
    }
    // Encode behind a reserved header so the frame leaves in one write: each
    // write on the vsock socket is a syscall.
    let mut encoded = vec![0_u8; 4];
    rmp_serde::encode::write_named(&mut encoded, frame).map_err(|error| invalid("encode exec frame", error))?;
    let len = u32::try_from(encoded.len() - 4).map_err(|error| invalid("measure exec frame", error))?;
    if len > MAX_EXEC_FRAME_BYTES {
        return Err(too_large("encoded exec frame"));
    }
    encoded[..4].copy_from_slice(&len.to_be_bytes());
    writer.write_all(&encoded)
}

fn read_frame<T: DeserializeOwned>(reader: &mut impl Read) -> io::Result<T> {
    let mut header = [0_u8; 4];
    reader.read_exact(&mut header)?;
    let len = u32::from_be_bytes(header);
    if len > MAX_EXEC_FRAME_BYTES {
        return Err(too_large("encoded exec frame"));
    }
    let mut payload = vec![0_u8; len as usize];
    reader.read_exact(&mut payload)?;
    rmp_serde::from_slice(&payload).map_err(|error| invalid("decode exec frame", error))
}

fn too_large(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("{what} exceeds its bound"))
}

fn invalid(context: &str, error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("{context}: {error}"))
}

#[cfg(test)]
mod tests;
