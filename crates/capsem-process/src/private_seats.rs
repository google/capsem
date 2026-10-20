//! Binding this owner's private seats at start: the cables holding each
//! network's guest stream, and the socket the service asks a cable's stream
//! on.
//! It needs the service's socket and the run directory the service named.
use crate::job_store::JobStore;
use anyhow::{Context, Result};
use capsem_proto::ipc::ServiceToProcess;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;

pub(crate) struct Seats<'a> {
    pub id: &'a str,
    pub service_socket: Option<&'a Path>,
    pub uds_path: &'a Path,
    pub run_dir: Option<&'a Path>,
}

fn bound(path: std::path::PathBuf, what: &str) -> Result<(std::path::PathBuf, std::os::unix::net::UnixListener)> {
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    let listener = std::os::unix::net::UnixListener::bind(&path).with_context(|| format!("bind {what} socket"))?;
    listener.set_nonblocking(true)?;
    std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    Ok((path, listener))
}

/// What the seats were bound with, for the other things this owner asks
/// the service on its VM's behalf.
pub(crate) struct Bound {
    cables: Arc<crate::cables::Cables>,
    seat_listener: tokio::net::UnixListener,
}

impl Bound {
    pub(crate) fn start(self) {
        tokio::spawn(self.cables.serve_seat(self.seat_listener));
    }
}

pub(crate) struct Prepared {
    pub service_socket: std::path::PathBuf,
    seat_path: std::path::PathBuf,
    seat_listener: std::os::unix::net::UnixListener,
}

impl Prepared {
    pub(crate) fn activate(self, job_store: &Arc<JobStore>, control: mpsc::Sender<ServiceToProcess>) -> Result<Bound> {
        let cables = Arc::new(crate::cables::Cables::new(job_store.publisher.clone(), control));
        let _ = job_store.cables.set(Arc::clone(&cables));
        let _ = job_store.cable_seat.set(self.seat_path);
        Ok(Bound {
            cables,
            seat_listener: tokio::net::UnixListener::from_std(self.seat_listener)?,
        })
    }
}

pub(crate) fn prepare(seats: Seats<'_>) -> Result<Prepared> {
    // The run directory the service named; otherwise where the service put
    // our IPC socket, walked up.
    let walked_up = seats
        .uds_path
        .parent()
        .and_then(|instances| instances.parent())
        .unwrap_or_else(|| Path::new("/tmp"))
        .to_path_buf();
    let run_dir = seats.run_dir.map(Path::to_path_buf).unwrap_or(walked_up);
    let service_socket = seats
        .service_socket
        .map(Path::to_path_buf)
        .unwrap_or_else(|| run_dir.join("service.sock"));

    let (seat_path, seat_listener) = bound(
        capsem_foundation::uds::private_handoff_socket_path(&run_dir, seats.id)?,
        "cable seat",
    )?;
    Ok(Prepared {
        service_socket,
        seat_path,
        seat_listener,
    })
}
