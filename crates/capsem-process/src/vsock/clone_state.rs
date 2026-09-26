//! A fork's clone, run by the sandbox's own process under a guest freeze.
//!
//! The guest freezes its system filesystem, the session is cloned on the
//! blocking pool, and the guest is thawed whatever the clone did: copying a
//! live ext4 overlay without the freeze can produce an image the fork cannot
//! boot, and running freeze and thaw in one process means no other process's
//! failure can leave the guest frozen. A guest that will not freeze gets no
//! fork. The service owns the destination and removes it when this fails.

use std::path::PathBuf;
use std::sync::Arc;

use capsem_proto::HostToGuest;
use tokio::sync::mpsc;

use crate::job_store::{with_quiescence, JobResult, JobStore};

/// How long a fork waits for the guest to freeze, as suspend does.
const CLONE_FREEZE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Copy the session, with every ledger row accepted so far on disk first.
///
/// The writer holds accepted rows in memory until its next disk flush, up to
/// five seconds away, and the clone copies the file: without this barrier a
/// fork left out whatever the source recorded in its last seconds (#243). Run
/// under the guest freeze, so nothing the guest does can land between the
/// flush and the copy. A flush that fails fails the fork rather than
/// producing a quietly incomplete one.
async fn flush_then_clone(db: &capsem_logger::DbWriter, source: PathBuf, destination: PathBuf) -> anyhow::Result<u64> {
    db.flush_checked()
        .await
        .map_err(|error| anyhow::anyhow!("flush the session ledger before cloning: {error}"))?;
    tokio::task::spawn_blocking(move || capsem_core::session::clone_sandbox_state(&source, &destination))
        .await
        .map_err(|error| anyhow::anyhow!("clone task failed: {error}"))?
}

/// Clone `source` into `destination` and answer job `id` with its size.
pub(super) fn spawn(
    hub_tx: &mpsc::Sender<HostToGuest>,
    job_store: &Arc<JobStore>,
    db: &Arc<capsem_logger::DbWriter>,
    source: &std::path::Path,
    id: u64,
    destination: String,
) {
    let (hub_tx, job_store, db, source) = (
        hub_tx.clone(),
        Arc::clone(job_store),
        Arc::clone(db),
        source.to_path_buf(),
    );
    tokio::spawn(async move {
        let destination = PathBuf::from(destination);
        let result = with_quiescence(&hub_tx, &job_store, CLONE_FREEZE_TIMEOUT, || {
            flush_then_clone(&db, source, destination)
        })
        .await
        .map_err(|error| format!("{error:#}"));
        if let Some(tx) = job_store.jobs.lock().unwrap().remove(&id) {
            capsem_core::try_send!("job_result_clone_state", tx.send(JobResult::CloneState { result }));
        }
    });
}

#[cfg(test)]
mod tests;
