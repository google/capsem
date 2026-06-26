//! Side-channel handshake for the typed bincode IPC channels.
//!
//! Run [`negotiate_initiator`] / [`negotiate_responder`] on a fresh
//! `std::os::unix::net::UnixStream` *before* handing it to
//! `tokio_unix_ipc::channel_from_std`. The handshake writes/reads a
//! length-prefixed `[u32 BE len][rmp-serde encoded Hello]` frame on the
//! raw socket. After both sides verify, ownership of the stream returns
//! to the caller and the bincode channel layer takes over.
//!
//! This is intentionally a side-channel rather than wrapping every
//! `Sender<T>` in `Frame<T>` because (a) it keeps the W1 try_send! sites
//! unchanged, (b) per-message traceparent override is W5 work, not W3,
//! and (c) the v0/v1 detection semantics are identical: a v0 binary that
//! never sends Hello times out within 5 seconds and produces a structured
//! `tracing::error!` line; a v1 binary at a different schema_hash fails
//! verify with both hashes in the log.
//!
//! Frame layout (matches the vsock control bridge's framing):
//! ```text
//! [4 bytes BE u32 length] [<length> bytes msgpack-serialized Hello]
//! ```
//!
//! The 5-second timeout is enforced via [`UnixStream::set_read_timeout`]
//! on the std socket.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use capsem_proto::handshake::{verify, HandshakeError, Hello};

/// Maximum bytes we'll allocate for a peer's Hello frame. A real Hello
/// is < 256 bytes; a hostile peer announcing a 4 GiB length must not be
/// able to OOM us before we drop the connection.
const MAX_HELLO_BYTES: u32 = 4096;

/// Default time we wait for the peer's Hello before declaring them
/// pre-handshake. 5 seconds is generous on macOS scheduling and the
/// handshake itself is trivially fast.
pub const HELLO_TIMEOUT: Duration = Duration::from_secs(5);

/// Initiator side: write our Hello, then read+verify the peer's. Returns
/// the peer's Hello on success (so the caller can stash its traceparent
/// for in-band propagation in W5).
///
/// On error the stream's pending writes/reads have already been observed,
/// so the caller should drop the stream rather than reuse it.
pub fn negotiate_initiator(
    stream: &mut UnixStream,
    peer_id: impl Into<String>,
    traceparent: impl Into<String>,
) -> Result<Hello, HandshakeError> {
    let prev_nb = ensure_blocking(stream);
    let result = (|| {
        write_hello(stream, &Hello::ours(peer_id, traceparent))?;
        let peer = read_hello(stream, HELLO_TIMEOUT)?;
        verify(&peer)?;
        Ok(peer)
    })();
    if let Some(true) = prev_nb {
        let _ = stream.set_nonblocking(true);
    }
    result
}

/// Responder side: read the peer's Hello with timeout, verify, then
/// reply with our own. Symmetric to the initiator.
pub fn negotiate_responder(
    stream: &mut UnixStream,
    peer_id: impl Into<String>,
    traceparent: impl Into<String>,
) -> Result<Hello, HandshakeError> {
    let prev_nb = ensure_blocking(stream);
    let result = (|| {
        let peer = read_hello(stream, HELLO_TIMEOUT)?;
        verify(&peer)?;
        write_hello(stream, &Hello::ours(peer_id, traceparent))?;
        Ok(peer)
    })();
    if let Some(true) = prev_nb {
        let _ = stream.set_nonblocking(true);
    }
    result
}

/// Force the stream into blocking mode for the handshake. Tokio's
/// `UnixStream::into_std()` returns a non-blocking std stream; the
/// std `read_exact` / `write_all` bail with WouldBlock instantly on
/// such streams, which manifested as "peer did not send Hello within
/// 5000ms" the first time we wired the handshake into the service.
/// Returns `Some(true)` if the stream WAS non-blocking (so the caller
/// can restore it), `Some(false)` if it was already blocking, `None`
/// if the query failed (treat as already-blocking).
fn ensure_blocking(stream: &UnixStream) -> Option<bool> {
    use std::os::unix::io::AsRawFd;
    let fd = stream.as_raw_fd();
    let was_nb = unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL, 0);
        if flags < 0 {
            return None;
        }
        Some((flags & libc::O_NONBLOCK) != 0)
    };
    let _ = stream.set_nonblocking(false);
    was_nb
}

fn write_hello(stream: &mut UnixStream, hello: &Hello) -> Result<(), HandshakeError> {
    let payload = rmp_serde::to_vec_named(hello)
        .map_err(|e| HandshakeError::Decode(format!("encode Hello: {e}")))?;
    let len = u32::try_from(payload.len())
        .map_err(|_| HandshakeError::Decode("Hello payload exceeds u32".into()))?;
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(&payload)?;
    stream.flush()?;
    Ok(())
}

fn read_hello(stream: &mut UnixStream, timeout: Duration) -> Result<Hello, HandshakeError> {
    let prev_timeout = stream.read_timeout().ok().flatten();
    stream.set_read_timeout(Some(timeout))?;

    let result = (|| {
        let mut len_buf = [0u8; 4];
        match stream.read_exact(&mut len_buf) {
            Ok(()) => {}
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                return Err(HandshakeError::Timeout {
                    timeout_ms: timeout.as_millis() as u64,
                });
            }
            Err(e) => return Err(HandshakeError::Io(e)),
        }

        let len = u32::from_be_bytes(len_buf);
        if len > MAX_HELLO_BYTES {
            return Err(HandshakeError::Decode(format!(
                "Hello length {len} exceeds {MAX_HELLO_BYTES}"
            )));
        }
        let mut buf = vec![0u8; len as usize];
        match stream.read_exact(&mut buf) {
            Ok(()) => {}
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                return Err(HandshakeError::Timeout {
                    timeout_ms: timeout.as_millis() as u64,
                });
            }
            Err(e) => return Err(HandshakeError::Io(e)),
        }

        rmp_serde::from_slice::<Hello>(&buf)
            .map_err(|e| HandshakeError::Decode(format!("decode Hello: {e}")))
    })();

    // Restore the previous timeout so the bincode channel that takes
    // over after handshake doesn't inherit our 5-second cap.
    let _ = stream.set_read_timeout(prev_timeout);
    result
}

#[cfg(test)]
mod tests;
