//! The bounded typed IPC transport between the service and each VM owner.
//!
//! Every message is one explicitly framed MessagePack value:
//! `[u32 payload_len_be][payload]`. The length is checked before allocation,
//! and the write half is locked across the complete frame so concurrent
//! callers cannot interleave bytes. Protocol compatibility is negotiated by
//! [`crate::ipc_handshake`] before this module takes ownership of the socket.
//!
//! [`Receiver::recv`] is cancel-safe: callers race it in `tokio::select!`, so
//! the bytes of a partially read frame are kept in the receiver and the next
//! call resumes where the dropped one stopped.

use std::fmt;
use std::io;
use std::marker::PhantomData;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;

use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::Mutex;

/// Largest service-to-owner or owner-to-service payload (32 MiB).
///
/// An exec completion can contain two 10 MiB lanes; HTTP events can likewise
/// contain a 10 MiB request and response. The remaining space covers the
/// typed envelope while keeping an unauthenticated local peer from announcing
/// an allocation anywhere near the old u32 maximum.
pub const MAX_IPC_FRAME_SIZE: u32 = 32 * 1024 * 1024;

/// A complete, bounded wire frame. Only the typed encoder can construct one.
pub struct PreparedFrame<T> {
    bytes: Vec<u8>,
    marker: PhantomData<fn(T)>,
}

/// The sending half of a typed channel.
pub struct Sender<T> {
    inner: Mutex<OwnedWriteHalf>,
    marker: PhantomData<fn(T)>,
}

/// The receiving half of a typed channel.
pub struct Receiver<T> {
    inner: Mutex<ReadState>,
    marker: PhantomData<fn() -> T>,
}

/// One frame's read progress, owned by the receiver rather than by a `recv`
/// future, so dropping that future loses no consumed bytes.
struct ReadState {
    half: OwnedReadHalf,
    header: [u8; 4],
    header_read: usize,
    payload: Vec<u8>,
    payload_read: usize,
    /// A rejected length leaves the stream unframed; every later call fails.
    unframed: bool,
}

impl ReadState {
    /// Fill `header` then `payload`, using only `read` (itself cancel-safe)
    /// and recording progress after every call.
    async fn next_frame(&mut self) -> io::Result<Vec<u8>> {
        if self.unframed {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "IPC stream lost framing"));
        }
        while self.header_read < self.header.len() {
            let count = self.half.read(&mut self.header[self.header_read..]).await?;
            if count == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            self.header_read += count;
            if self.header_read == self.header.len() {
                let len = u32::from_be_bytes(self.header);
                if len > MAX_IPC_FRAME_SIZE {
                    self.unframed = true;
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("IPC frame too large: {len} bytes (max {MAX_IPC_FRAME_SIZE})"),
                    ));
                }
                self.payload = vec![0_u8; len as usize];
                self.payload_read = 0;
            }
        }
        while self.payload_read < self.payload.len() {
            let count = self.half.read(&mut self.payload[self.payload_read..]).await?;
            if count == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            self.payload_read += count;
        }
        self.header_read = 0;
        Ok(std::mem::take(&mut self.payload))
    }
}

/// Register a connected stream with the current runtime as a typed channel
/// sending `S` and receiving `R`.
pub fn channel_from_std<S, R>(stream: UnixStream) -> io::Result<(Sender<S>, Receiver<R>)> {
    stream.set_nonblocking(true)?;
    let stream = tokio::net::UnixStream::from_std(stream)?;
    let (read, write) = stream.into_split();
    Ok((
        Sender {
            inner: Mutex::new(write),
            marker: PhantomData,
        },
        Receiver {
            inner: Mutex::new(ReadState {
                half: read,
                header: [0; 4],
                header_read: 0,
                payload: Vec::new(),
                payload_read: 0,
                unframed: false,
            }),
            marker: PhantomData,
        },
    ))
}

impl<T> Sender<T>
where
    T: Serialize,
{
    /// Serialize and send one complete bounded frame.
    pub async fn send(&self, message: T) -> io::Result<()> {
        self.send_prepared(Self::prepare(&message)?).await
    }

    /// Encode the final frame without socket I/O. The caller retains ownership
    /// of its message, so a rejected batch can be split without cloning bodies.
    pub fn prepare(message: &T) -> io::Result<PreparedFrame<T>> {
        // Encode once into the final frame. Reject excess bytes during encode,
        // before socket I/O; neither preflight serialization nor a payload copy.
        let mut frame = vec![0; 4];
        append_record(&mut frame, message, MAX_IPC_FRAME_SIZE as usize)?;
        let len = (frame.len() - 4) as u32;
        frame[..4].copy_from_slice(&len.to_be_bytes());
        Ok(PreparedFrame {
            bytes: frame,
            marker: PhantomData,
        })
    }

    /// Write an already encoded frame without another serialization or copy.
    pub async fn send_prepared(&self, frame: PreparedFrame<T>) -> io::Result<()> {
        self.inner.lock().await.write_all(&frame.bytes).await
    }
}

impl<T> Receiver<T>
where
    T: DeserializeOwned,
{
    /// Receive and decode one complete bounded frame. Cancel-safe.
    pub async fn recv(&self) -> io::Result<T> {
        let payload = self.inner.lock().await.next_frame().await?;
        decode_record(&payload, MAX_IPC_FRAME_SIZE as usize)
    }
}

/// Encode one typed, named record whose boundary is supplied by its atomic file.
/// The same encoder writes stream payloads directly after their reserved header.
pub fn encode_record<T: Serialize>(message: &T, limit: usize) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    append_record(&mut bytes, message, limit)?;
    Ok(bytes)
}

fn append_record<T: Serialize>(bytes: &mut Vec<u8>, message: &T, limit: usize) -> io::Result<()> {
    let mut writer = crate::limited_writer::LimitedWriter::new(bytes, limit);
    let encoded = rmp_serde::encode::write_named(&mut writer, message);
    let exceeded = writer.exceeded();
    encoded.map_err(|error| {
        if exceeded {
            invalid_data("MessagePack frame too large", limit)
        } else {
            invalid_data("encode MessagePack record", error)
        }
    })
}

/// Decode one bounded record without copying its encoded bytes.
pub fn decode_record<T: DeserializeOwned>(bytes: &[u8], limit: usize) -> io::Result<T> {
    if bytes.len() > limit {
        return Err(invalid_data("MessagePack frame too large", limit));
    }
    rmp_serde::from_slice(bytes).map_err(|error| invalid_data("decode MessagePack record", error))
}

fn invalid_data(context: &str, error: impl fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("{context}: {error}"))
}

impl<T> fmt::Debug for Sender<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let fd = self.inner.try_lock().ok().map(|inner| inner.as_ref().as_raw_fd());
        formatter.debug_struct("Sender").field("fd", &fd).finish()
    }
}

impl<T> fmt::Debug for Receiver<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let fd = self.inner.try_lock().ok().map(|inner| inner.half.as_ref().as_raw_fd());
        formatter.debug_struct("Receiver").field("fd", &fd).finish()
    }
}

#[cfg(test)]
mod tests;
