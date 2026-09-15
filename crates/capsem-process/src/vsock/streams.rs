//! Guest byte streams handed whole to an async handler: the MITM rail and
//! each network cable's stream. Both duplicate the accepted descriptor and
//! keep the connection alive for as long as the handler runs.
use capsem_core::VsockConnection;
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

/// A guest pump's stream for one cable, held for that network's switch once
/// the pump has named the cable. The four header bytes are read exactly and
/// never buffered: everything after them belongs to the switch.
pub(super) fn serve_network(conn: VsockConnection, job_store: &Arc<crate::job_store::JobStore>, vm_id: &str) {
    let Some(cables) = job_store.cables.get().cloned() else {
        warn!(vm = %vm_id, "network: cable stream refused; this owner has no cables");
        return;
    };
    let vm_id = vm_id.to_string();
    tokio::spawn(async move {
        match read_cable_header(&conn).await {
            Ok(cable) => {
                info!(vm = %vm_id, cable, "network: cable stream attached");
                cables.attach_guest(cable, conn);
            }
            Err(error) => warn!(vm = %vm_id, %error, "network: cable stream refused"),
        }
    });
}

async fn read_cable_header(conn: &VsockConnection) -> Result<u32, String> {
    use capsem_proto::privatelink::{decode_cable_header, CABLE_HEADER_BYTES};
    use tokio::io::AsyncReadExt;
    let fd = conn
        .try_clone_fd()
        .map_err(|error| format!("duplicate the stream: {error}"))?;
    let std = std::os::unix::net::UnixStream::from(fd);
    std.set_nonblocking(true).map_err(|error| error.to_string())?;
    let mut stream = tokio::net::UnixStream::from_std(std).map_err(|error| error.to_string())?;
    let mut header = [0u8; CABLE_HEADER_BYTES];
    tokio::time::timeout(std::time::Duration::from_secs(2), stream.read_exact(&mut header))
        .await
        .map_err(|_| "no cable header within two seconds".to_string())?
        .map_err(|error| format!("read the cable header: {error}"))?;
    // Dropping the duplicate closes it and never shuts the connection down:
    // the connection is the cable's from here (S04-018).
    drop(stream);
    decode_cable_header(&header)
}

#[cfg(test)]
mod tests;
