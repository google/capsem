//! Correlated guest filesystem barriers, serialized by the lifecycle owner.
use super::*;
use anyhow::Context;
use std::time::Duration;
use tokio::sync::mpsc;

pub(crate) struct Waiter {
    pub(super) id: u64,
    pub(super) frozen: bool,
    pub(super) reply: oneshot::Sender<Result<(), String>>,
}

struct Waiting<'a> {
    jobs: &'a JobStore,
    id: u64,
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        let mut pending = self.jobs.snapshot_transition.lock().unwrap();
        if pending.as_ref().is_some_and(|pending| pending.id == self.id) {
            *pending = None;
        }
    }
}

impl JobStore {
    pub(crate) fn finish_snapshot_transition(&self, id: u64, frozen: bool, error: Option<String>) {
        let reply = {
            let mut pending = self.snapshot_transition.lock().unwrap();
            let reply = if pending
                .as_ref()
                .is_some_and(|pending| pending.id == id && pending.frozen == frozen)
            {
                pending.take().map(|pending| pending.reply)
            } else {
                None
            };
            drop(pending);
            reply
        };
        if let Some(reply) = reply {
            let _ = reply.send(error.map_or(Ok(()), Err));
        }
    }
}

/// The caller holds quiescence through freeze, VM work and thaw. Each phase
/// has a fresh identity; a late acknowledgement cannot satisfy its successor.
pub(crate) async fn snapshot_transition(
    host: &mpsc::Sender<HostToGuest>,
    jobs: &JobStore,
    bound: Duration,
    frozen: bool,
) -> Result<()> {
    let id = jobs.next_control_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let (reply, received) = oneshot::channel();
    {
        let mut pending = jobs.snapshot_transition.lock().unwrap();
        anyhow::ensure!(pending.is_none(), "another snapshot transition is pending");
        *pending = Some(Waiter { id, frozen, reply });
    }
    let _waiting = Waiting { jobs, id };
    tokio::time::timeout(bound, async {
        host.send(HostToGuest::SnapshotTransition { id, frozen })
            .await
            .context("send snapshot transition")?;
        received
            .await
            .context("snapshot transition channel closed prematurely")?
            .map_err(anyhow::Error::msg)
    })
    .await
    .context("snapshot transition timed out")?
}

/// Copies run while the VM is running. Both successful and rejected copies
/// must wait for the guest to confirm thaw before returning to the service.
pub(crate) async fn with_quiescence<T, F, Fut>(
    host: &mpsc::Sender<HostToGuest>,
    jobs: &Arc<JobStore>,
    bound: Duration,
    operation: F,
) -> Result<T>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let _sequence = jobs.quiescence.lock().await;
    let result = match snapshot_transition(host, jobs, bound, true).await {
        Ok(()) => operation().await,
        Err(error) => Err(error),
    };
    let thaw = snapshot_transition(host, jobs, bound, false).await;
    match (result, thaw) {
        (result, Ok(())) => result,
        (Ok(_), Err(thaw)) => Err(thaw.context("thaw guest filesystem after copy")),
        (Err(error), Err(thaw)) => Err(error.context(format!("guest thaw also failed: {thaw:#}"))),
    }
}
