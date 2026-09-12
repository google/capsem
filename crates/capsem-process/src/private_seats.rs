//! Binding this owner's private seat at start: the handoff socket other
//! owners deliver TCP streams to and the service asks the guest link from.
//! It needs the service's socket, the run directory the service named and
//! the secret it minted for this VM.
use crate::job_store::JobStore;
use anyhow::{Context, Result};
use capsem_proto::ipc::ServiceToProcess;
use std::path::Path;
use std::sync::Arc;
use tokio::net::UnixListener;
use tokio::sync::mpsc;

pub(crate) struct Seats<'a> {
    pub id: &'a str,
    pub service_socket: Option<&'a Path>,
    pub uds_path: &'a Path,
    pub run_dir: Option<&'a Path>,
    pub session_dir: &'a Path,
}

fn bound(path: std::path::PathBuf, what: &str) -> Result<(std::path::PathBuf, UnixListener)> {
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    let listener = UnixListener::bind(&path).with_context(|| format!("bind {what} socket"))?;
    std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    Ok((path, listener))
}

pub(crate) fn bind(seats: Seats<'_>, job_store: &Arc<JobStore>, control: mpsc::Sender<ServiceToProcess>) -> Result<()> {
    // The run directory the service named; otherwise where the service put
    // our IPC socket, walked up.
    let walked_up = seats
        .uds_path
        .parent()
        .and_then(|instances| instances.parent())
        .unwrap_or_else(|| Path::new("/tmp"))
        .to_path_buf();
    let run_dir = seats.run_dir.map(Path::to_path_buf).unwrap_or(walked_up);
    let owner_secret = std::fs::read_to_string(seats.session_dir.join("owner-secret"))
        .map(|secret| secret.trim().to_string())
        .unwrap_or_default();
    let service_socket = seats
        .service_socket
        .map(Path::to_path_buf)
        .unwrap_or_else(|| run_dir.join("service.sock"));

    let (handoff_path, handoff_listener) = bound(
        capsem_foundation::uds::private_handoff_socket_path(&run_dir, seats.id)?,
        "private handoff",
    )?;
    let handoff = Arc::new(crate::private_handoff::PrivateHandoff::new(
        handoff_path,
        job_store.publisher.clone(),
        control,
        service_socket,
        owner_secret,
        seats.id.to_string(),
    ));
    let _ = job_store.private.set(Arc::clone(&handoff));
    tokio::spawn(handoff.serve(handoff_listener));

    Ok(())
}
