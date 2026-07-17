use anyhow::{Context, Result};
use axum::extract::ws::{Message, WebSocket};
use bytes::BytesMut;
use capsem_core::hypervisor::VmHandle;
use capsem_core::VsockConnection;
use capsem_proto::GUI_VSOCK_PORT;
use futures::{SinkExt, StreamExt};
use std::os::fd::{FromRawFd, RawFd};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;
use tracing::{info, warn};

/// Xpra packets can contain full desktop frames. Keep the WebSocket bound
/// explicit and high enough for one uncompressed 1200x800 RGBA frame while
/// still preventing a client from asking capsem-process to buffer arbitrarily.
pub(crate) const GUI_WS_MAX_MESSAGE_BYTES: usize = 64 * 1024 * 1024;

/// One slab holds a complete uncompressed 3840x2160 RGBA frame. WebSocket
/// message boundaries remain transport details: Xpra parses its byte stream.
const GUI_RELAY_CHUNK_BYTES: usize = 32 * 1024 * 1024;

/// Connect the process-owned private WebSocket to Xpra's fixed guest vsock
/// listener. The browser controls neither the vsock port nor a guest command.
pub(crate) async fn handle_gui_socket(socket: WebSocket, vm: Arc<Mutex<Box<dyn VmHandle>>>) {
    let connection = match connect_gui_vsock(vm).await {
        Ok(connection) => connection,
        Err(error) => {
            warn!(%error, port = GUI_VSOCK_PORT, "GUI vsock connect failed");
            return;
        }
    };

    let stream = match duplicate_connection_stream(connection.fd) {
        Ok(stream) => stream,
        Err(error) => {
            warn!(%error, port = GUI_VSOCK_PORT, "GUI vsock stream setup failed");
            return;
        }
    };

    info!(
        port = GUI_VSOCK_PORT,
        "GUI WebSocket connected to guest Xpra vsock"
    );
    relay_gui_stream(socket, stream).await;

    // Keep the VZ connection anchor alive until both relay halves have ended.
    drop(connection);
    info!(port = GUI_VSOCK_PORT, "GUI WebSocket disconnected");
}

async fn connect_gui_vsock(vm: Arc<Mutex<Box<dyn VmHandle>>>) -> Result<VsockConnection> {
    tokio::task::spawn_blocking(move || {
        let vm = vm.blocking_lock();
        vm.connect_vsock(GUI_VSOCK_PORT)
    })
    .await
    .context("GUI vsock connector task failed")?
}

fn duplicate_connection_stream(fd: RawFd) -> Result<tokio::net::UnixStream> {
    let duplicate = nix::unistd::dup(fd).context("duplicate GUI vsock fd")?;
    // SAFETY: dup returned a new owned descriptor. std_stream is its sole
    // owner, so this cannot double-close the VZ-owned original descriptor.
    let std_stream = unsafe { std::os::unix::net::UnixStream::from_raw_fd(duplicate) };
    std_stream
        .set_nonblocking(true)
        .context("set GUI vsock duplicate nonblocking")?;
    tokio::net::UnixStream::from_std(std_stream).context("register GUI vsock stream with Tokio")
}

async fn relay_gui_stream(socket: WebSocket, stream: tokio::net::UnixStream) {
    let (mut browser_write, mut browser_read) = socket.split();
    let (mut guest_read, mut guest_write) = stream.into_split();

    let browser_to_guest = async {
        while let Some(message) = browser_read.next().await {
            match message {
                Ok(Message::Binary(bytes)) => {
                    if guest_write.write_all(&bytes).await.is_err() {
                        break;
                    }
                }
                Ok(Message::Close(_)) | Err(_) => break,
                // Xpra is a binary protocol. Ping/Pong are handled by axum;
                // text and other application frames are not forwarded.
                _ => {}
            }
        }
        let _ = guest_write.shutdown().await;
    };

    let guest_to_browser = async {
        let mut buffer = BytesMut::with_capacity(GUI_RELAY_CHUNK_BYTES);
        loop {
            buffer.reserve(GUI_RELAY_CHUNK_BYTES);
            match guest_read.read_buf(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    // split().freeze() transfers the allocation into the
                    // WebSocket frame without copying the Xpra bytes.
                    let bytes = buffer.split().freeze();
                    if browser_write.send(Message::Binary(bytes)).await.is_err() {
                        break;
                    }
                }
            }
        }
    };

    tokio::pin!(browser_to_guest);
    tokio::pin!(guest_to_browser);
    tokio::select! {
        _ = &mut browser_to_guest => {}
        _ = &mut guest_to_browser => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;

    #[tokio::test]
    async fn duplicated_stream_owns_a_distinct_working_fd() {
        let (left, mut right) = tokio::net::UnixStream::pair().unwrap();
        let duplicate = duplicate_connection_stream(left.as_raw_fd()).unwrap();
        drop(left);

        let (_, mut duplicate_write) = duplicate.into_split();
        duplicate_write.write_all(b"xpra").await.unwrap();

        let mut received = [0_u8; 4];
        right.read_exact(&mut received).await.unwrap();
        assert_eq!(&received, b"xpra");
    }

    #[test]
    fn websocket_message_bound_covers_one_rgba_spike_frame() {
        let rgba_4k_frame = 3840 * 2160 * 4;
        assert!(GUI_RELAY_CHUNK_BYTES >= rgba_4k_frame);
        assert!(GUI_WS_MAX_MESSAGE_BYTES >= GUI_RELAY_CHUNK_BYTES);
        assert!(GUI_WS_MAX_MESSAGE_BYTES <= 64 * 1024 * 1024);
    }
}
