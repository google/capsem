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

/// The guest's tun0 packet stream, terminated in smoltcp inside this process
/// for the S04-004 measurement; the confined capsem-network process of
/// S04-002 takes the descriptor instead.
pub(super) fn serve_network(conn: VsockConnection, vm_id: &str) {
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
