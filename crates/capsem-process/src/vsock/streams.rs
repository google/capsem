//! Guest byte streams handed whole to an async handler: the MITM rail and
//! the tun0 packet stream. Both duplicate the accepted descriptor and keep
//! the connection alive for as long as the handler runs.
use capsem_core::VsockConnection;
use capsem_proto::privatelink::{ConnectHeader, HEADER_BYTES};
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
pub(super) fn serve(conn: VsockConnection, job_store: &Arc<crate::job_store::JobStore>, vm_id: &str) {
    match capsem_proto::HostVsockService::from_port(conn.port) {
        Some(capsem_proto::HostVsockService::Private) => serve_private(conn, Arc::clone(job_store), vm_id),
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

/// A guest connection to a private address: the header names where it was
/// going, the meta line who asked. The owner asks the service and, granted,
/// delivers the stream to the destination owner; any failure before that
/// closes the guest's connection without a byte crossing, so a connect to a
/// non-member fails fast instead of hanging.
/// The bytes before a private connection's payload: the connect header, then
/// the process meta line. Read exactly, never buffered: the descriptor is
/// handed to another owner next, and a payload byte read here would be a
/// byte the destination never sees. A client that writes its first request
/// in the same segment as the preamble, as the throughput client does, lost
/// it to a BufReader and waited for an answer that could not come.
pub(super) async fn read_private_preamble<R>(reader: &mut R) -> Result<(ConnectHeader, String), String>
where
    R: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::AsyncReadExt;
    let mut header = [0u8; HEADER_BYTES];
    reader
        .read_exact(&mut header)
        .await
        .map_err(|error| error.to_string())?;
    let header = ConnectHeader::decode(&header)?;
    let mut meta = Vec::new();
    loop {
        let byte = reader.read_u8().await.map_err(|error| error.to_string())?;
        if byte == b'\n' {
            break;
        }
        meta.push(byte);
        if meta.len() > META_LINE_MAX {
            return Err("process meta line too long".into());
        }
    }
    let name = String::from_utf8_lossy(&meta);
    let name = name
        .trim_start_matches('\0')
        .strip_prefix("CAPSEM_META:")
        .unwrap_or("unknown")
        .trim()
        .to_string();
    Ok((header, name))
}

/// The meta line is a process name; anything longer is not one.
const META_LINE_MAX: usize = 256;

fn serve_private(conn: VsockConnection, job_store: Arc<crate::job_store::JobStore>, vm_id: &str) {
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
        let preamble = tokio::time::timeout(std::time::Duration::from_secs(2), read_private_preamble(&mut stream))
            .await
            .map_err(|_| "header timed out".to_string())
            .and_then(|preamble| preamble);
        let (header, process_name) = match preamble {
            Ok(preamble) => preamble,
            Err(error) => {
                warn!(vm = %vm, error = %error, "private connection header rejected");
                return;
            }
        };
        let Some(handoff) = job_store.private.get() else {
            warn!(vm = %vm, destination = %header.destination, port = header.port, "private connection refused: no handoff on this owner");
            return;
        };
        match handoff.connect_out(conn, header, process_name).await {
            Ok(()) => {
                info!(vm = %vm, destination = %header.destination, port = header.port, "private connection ended")
            }
            Err(error) => {
                info!(vm = %vm, destination = %header.destination, port = header.port, %error, "private connection refused");
            }
        }
    });
}

#[cfg(test)]
mod tests;
