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

/// Clone `source` into `destination` and answer job `id` with its size.
pub(super) fn spawn(
    hub_tx: &mpsc::Sender<HostToGuest>,
    job_store: &Arc<JobStore>,
    source: &std::path::Path,
    id: u64,
    destination: String,
) {
    let (hub_tx, job_store, source) = (hub_tx.clone(), Arc::clone(job_store), source.to_path_buf());
    tokio::spawn(async move {
        let destination = PathBuf::from(destination);
        let result = with_quiescence(&hub_tx, &job_store, CLONE_FREEZE_TIMEOUT, || async {
            tokio::task::spawn_blocking(move || capsem_core::session::clone_sandbox_state(&source, &destination))
                .await
                .map_err(|error| anyhow::anyhow!("clone task failed: {error}"))?
        })
        .await
        .map_err(|error| format!("{error:#}"));
        if let Some(tx) = job_store.jobs.lock().unwrap().remove(&id) {
            capsem_core::try_send!("job_result_clone_state", tx.send(JobResult::CloneState { result }));
        }
    });
}
