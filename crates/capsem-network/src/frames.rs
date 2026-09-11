//! IP packets on a byte stream: a big-endian `u16` length, then the packet.
//!
//! Both ends of the VSOCK connection speak this and nothing else. The guest
//! pump in `capsem-agent` has the same two functions in blocking form; a
//! change here is a change there.
use std::io;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// The header is two bytes, so a frame is at most 65535 bytes of packet.
pub const MAX_FRAME_BYTES: usize = u16::MAX as usize;
pub const HEADER_BYTES: usize = 2;

/// Read one frame into `packet`, which is resized to the packet length.
///
/// `Ok(None)` is a clean end of stream between frames; an end of stream
/// inside a frame is an error, because a packet the peer had started is a
/// packet it did not finish.
pub async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R, packet: &mut Vec<u8>) -> io::Result<Option<usize>> {
    let mut header = [0u8; HEADER_BYTES];
    match reader.read_exact(&mut header).await {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let length = usize::from(u16::from_be_bytes(header));
    if length == 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "empty frame"));
    }
    packet.resize(length, 0);
    reader.read_exact(&mut packet[..length]).await?;
    Ok(Some(length))
}

/// Write one packet as a frame. Refuses a packet the header cannot describe.
pub async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, packet: &[u8]) -> io::Result<()> {
    if packet.is_empty() || packet.len() > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("frame of {} bytes cannot be described by a u16 header", packet.len()),
        ));
    }
    let header = (packet.len() as u16).to_be_bytes();
    writer.write_all(&header).await?;
    writer.write_all(packet).await
}

#[cfg(test)]
mod tests;
