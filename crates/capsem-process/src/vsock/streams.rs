//! Guest byte streams handed whole to an async handler: the MITM rail and
//! the tun0 packet stream. Both duplicate the accepted descriptor and keep
//! the connection alive for as long as the handler runs.
use capsem_core::VsockConnection;
use std::os::fd::AsFd;
use std::sync::Arc;
use tracing::{error, info, warn};

pub(super) fn serve_mitm(conn: VsockConnection, config: Arc<capsem_core::net::mitm_proxy::MitmProxyConfig>) {
    tokio::spawn(async move {
        match conn.try_clone_fd() {
            Ok(fd) => capsem_core::net::mitm_proxy::handle_connection(fd, config).await,
            Err(error) => error!(
                operation = "duplicate-mitm-vsock",
                errno = error.raw_os_error(),
                error = %error,
                "MITM connection descriptor unavailable"
            ),
        }
    });
}

/// The two guest streams this module owns, by the port they arrived on.
pub(super) fn serve(service: capsem_proto::HostVsockService, conn: VsockConnection, vm_id: &str) {
    match service {
        capsem_proto::HostVsockService::Private => serve_private(conn, vm_id),
        _ => serve_network(conn, vm_id),
    }
}

/// The guest's tun0 packet stream, terminated in smoltcp inside this process
/// for the S04-004 measurement; the confined capsem-network process of
/// S04-002 takes the descriptor instead.
fn serve_network(conn: VsockConnection, vm_id: &str) {
    let vm = vm_id.to_string();
    tokio::spawn(async move {
        let stream = conn.try_clone_fd().and_then(|fd| {
            capsem_foundation::unix::fd::set_nonblocking(fd.as_fd(), true)?;
            tokio::net::UnixStream::from_std(std::os::unix::net::UnixStream::from(fd))
        });
        let stream = match stream {
            Ok(stream) => stream,
            Err(error) => {
                error!(
                    operation = "duplicate-network-vsock",
                    errno = error.raw_os_error(),
                    error = %error,
                    "network packet stream descriptor unavailable"
                );
                return;
            }
        };
        info!(vm = %vm, "network: guest tun0 packet stream attached");
        match capsem_network::serve_throughput(stream).await {
            Ok(_) => info!(vm = %vm, "network: guest tun0 packet stream ended"),
            Err(error) => warn!(vm = %vm, error = %error, "network: guest tun0 packet stream failed"),
        }
        drop(conn);
    });
}

/// A guest connection to a private address. The header names where it was
/// going; until the service admits private connections (S04-016) the owner
/// refuses every one before any payload byte, and says so with the
/// destination, so a guest connect fails fast instead of hanging.
fn serve_private(conn: VsockConnection, vm_id: &str) {
    use capsem_proto::privatelink::{ConnectHeader, HEADER_BYTES};
    use tokio::io::AsyncReadExt;
    let vm = vm_id.to_string();
    tokio::spawn(async move {
        let stream = conn.try_clone_fd().and_then(|fd| {
            capsem_foundation::unix::fd::set_nonblocking(fd.as_fd(), true)?;
            tokio::net::UnixStream::from_std(std::os::unix::net::UnixStream::from(fd))
        });
        let mut stream = match stream {
            Ok(stream) => stream,
            Err(error) => {
                error!(vm = %vm, errno = error.raw_os_error(), error = %error, "private connection descriptor unavailable");
                return;
            }
        };
        let mut header = [0u8; HEADER_BYTES];
        let read = tokio::time::timeout(std::time::Duration::from_secs(2), stream.read_exact(&mut header)).await;
        match read.map_err(|_| "header timed out".to_string()).and_then(|read| {
            read.map_err(|error| error.to_string())?;
            ConnectHeader::decode(&header)
        }) {
            Ok(header) => info!(
                vm = %vm, destination = %header.destination, port = header.port,
                "private connection refused: no private path yet"
            ),
            Err(error) => warn!(vm = %vm, error = %error, "private connection header rejected"),
        }
        drop(stream);
        drop(conn);
    });
}
