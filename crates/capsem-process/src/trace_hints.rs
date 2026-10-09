//! Receiver for the proxy's one-way model/file correlation capability.

use std::io;
use std::os::fd::AsFd as _;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};

use capsem_core::net::ai_traffic::TraceState;
use capsem_proto::proxy_trace_hints::{decode_proxy_trace_hint, PROXY_TRACE_HINT_ACK, PROXY_TRACE_HINT_FRAME_SIZE};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

pub(crate) async fn serve(socket: UnixStream, trace_state: Arc<Mutex<TraceState>>) -> io::Result<()> {
    capsem_foundation::unix::fd::set_nonblocking(socket.as_fd(), true)?;
    let mut socket = tokio::net::UnixStream::from_std(socket)?;
    loop {
        let mut frame = [0; PROXY_TRACE_HINT_FRAME_SIZE];
        match socket.read_exact(&mut frame).await {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(error) => return Err(error),
        }
        let hint =
            decode_proxy_trace_hint(&frame).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if !trace_state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .register_file_hint(&hint.trace_id, &hint.relative_path)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "proxy trace hint names an invalid workspace path",
            ));
        }
        socket.write_all(&PROXY_TRACE_HINT_ACK).await?;
    }
}

#[cfg(test)]
mod tests;
