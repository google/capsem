//! Hold a guest filesystem freeze around a coordinator-owned session copy.
//!
//! The VM owner controls freeze and thaw because it owns the guest control
//! channel. It receives no source or destination path and copies no host
//! state. The coordinator reports the copy result, then the owner thaws the
//! guest before answering the original request.

use std::sync::Arc;

use capsem_proto::{ipc::ProcessToService, HostToGuest};
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::job_store::{with_quiescence, JobResult, JobStore};

const CLONE_FREEZE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const COORDINATOR_COPY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(900);

async fn await_coordinator_copy(
    jobs: &JobStore,
    events: &broadcast::Sender<ProcessToService>,
    id: u64,
) -> anyhow::Result<u64> {
    let (completed, result) = oneshot::channel();
    if jobs.clone_completions.lock().unwrap().insert(id, completed).is_some() {
        anyhow::bail!("clone {id} already awaits coordinator completion");
    }
    if events.send(ProcessToService::CloneStateReady { id }).is_err() {
        jobs.clone_completions.lock().unwrap().remove(&id);
        anyhow::bail!("clone {id} has no coordinator listener");
    }
    match tokio::time::timeout(COORDINATOR_COPY_TIMEOUT, result).await {
        Ok(Ok(result)) => result.map_err(anyhow::Error::msg),
        Ok(Err(_)) => anyhow::bail!("clone {id} coordinator completion channel closed"),
        Err(_) => {
            jobs.clone_completions.lock().unwrap().remove(&id);
            anyhow::bail!("clone {id} coordinator copy timed out")
        }
    }
}

pub(super) fn spawn(
    hub_tx: &mpsc::Sender<HostToGuest>,
    job_store: &Arc<JobStore>,
    events: &broadcast::Sender<ProcessToService>,
    id: u64,
) {
    let (hub_tx, job_store, events) = (hub_tx.clone(), Arc::clone(job_store), events.clone());
    tokio::spawn(async move {
        let result = with_quiescence(&hub_tx, &job_store, CLONE_FREEZE_TIMEOUT, || {
            await_coordinator_copy(&job_store, &events, id)
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
