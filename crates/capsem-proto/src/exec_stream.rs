//! Bounded bidirectional frames on the dedicated guest exec VSOCK.

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::io::{self, Read, Write};

/// Largest stdin or output chunk carried by one exec frame.
pub const MAX_EXEC_DATA_BYTES: usize = 256 * 1024;
/// Stdin frames (data or EOF) one streaming exec may have in flight between
/// the service and the guest. The VM owner queues at most this many and
/// reports each one it hands to the guest with
/// `ProcessToService::ExecInputConsumed`; the service sends a
/// frame only while it holds credit, so neither side ever waits on the other.
pub const EXEC_STDIN_WINDOW: usize = 16;
/// Largest encoded exec frame. The typed envelope stays well below this when
/// its data is at [`MAX_EXEC_DATA_BYTES`].
const MAX_EXEC_FRAME_BYTES: u32 = 512 * 1024;

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
    write_frame(writer, frame, frame.data.len())
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
    let payload = rmp_serde::to_vec_named(frame).map_err(|error| invalid("encode exec frame", error))?;
    let len = u32::try_from(payload.len()).map_err(|error| invalid("measure exec frame", error))?;
    if len > MAX_EXEC_FRAME_BYTES {
        return Err(too_large("encoded exec frame"));
    }
    writer.write_all(&len.to_be_bytes())?;
    writer.write_all(&payload)
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
