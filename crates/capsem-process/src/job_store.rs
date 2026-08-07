use anyhow::Result;
use capsem_proto::HostToGuest;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::{oneshot, Notify};
use tracing::{info, warn};

pub(crate) struct JobStore {
    pub(crate) jobs: Mutex<HashMap<u64, oneshot::Sender<JobResult>>>,
    /// Currently active exec job: the id, captured stdout, and a notifier
    /// the EXEC-port reader thread fires once it has finished depositing
    /// captured bytes. The ExecDone handler awaits this before reading
    /// captured, so no-output commands don't pay a blanket timeout to
    /// cover the deposit race.
    pub(crate) active_exec: Mutex<Option<ActiveExec>>,
    /// In-flight explicit file operations keyed by service job id. The guest
    /// response only carries enough context for reads; writes need the original
    /// path and payload to emit the security-event ledger row after success.
    pub(crate) active_file_ops: Mutex<HashMap<u64, ActiveFileOp>>,
    /// Channel for snapshot ready signal.
    pub(crate) snapshot_ready: Mutex<Option<oneshot::Sender<()>>>,
    /// Pending-ack map for the control bridge's reliability layer:
    /// every ackable HostToGuest (Exec / FileWrite / FileRead /
    /// FileDelete) we write goes here keyed by `id`. The agent sends
    /// `GuestToHost::Ack { id }` immediately on receipt; the bridge
    /// removes the entry. On rekey we replay every entry still in the
    /// map. Lives on `JobStore` so IPC handlers can also remove entries
    /// once no caller is still waiting (for example quick file-operation
    /// watchdog exhaustion), keeping the map bounded even when the agent
    /// never acks. See `vsock.rs::setup_vsock` for the bridge end and
    /// `ipc.rs::handle_ipc_connection` for the IPC end.
    pub(crate) pending_acks: Mutex<HashMap<u64, HostToGuest>>,
}

/// State for an in-flight exec. `deposited` is notified once by the
/// EXEC-port reader thread after it has written `captured` under the
/// active_exec lock; ExecDone uses it to distinguish "empty stdout" from
/// "deposit still in flight" without sleeping unconditionally.
pub(crate) struct ActiveExec {
    pub(crate) id: u64,
    pub(crate) started_at: Instant,
    pub(crate) event_id: Option<capsem_core::security_engine::SecurityEventId>,
    pub(crate) captured: Vec<u8>,
    /// Bytes the guest actually wrote, which exceeds `captured.len()` when the
    /// output was capped. Telemetry reports this so `stdout_bytes` stays the
    /// real volume rather than the retained slice.
    pub(crate) total_bytes: u64,
    pub(crate) deposited: Arc<Notify>,
}

impl ActiveExec {
    pub(crate) fn new(id: u64) -> Self {
        Self {
            id,
            started_at: Instant::now(),
            event_id: None,
            captured: Vec::new(),
            total_bytes: 0,
            deposited: Arc::new(Notify::new()),
        }
    }
}

impl JobStore {
    pub(crate) fn new() -> Self {
        Self {
            jobs: Mutex::new(HashMap::new()),
            active_exec: Mutex::new(None),
            active_file_ops: Mutex::new(HashMap::new()),
            snapshot_ready: Mutex::new(None),
            pending_acks: Mutex::new(HashMap::new()),
        }
    }

    /// Drain every pending job and oneshot, answering each with an Error.
    /// Called when the control-channel reader has died so callers waiting on
    /// the oneshots (see `ipc.rs` Exec/WriteFile/ReadFile handlers) get a
    /// prompt failure instead of a 30s IPC timeout.
    pub(crate) fn fail_all(&self, message: &str) {
        let pending: Vec<_> = self.jobs.lock().unwrap().drain().collect();
        for (_id, tx) in pending {
            capsem_core::try_send!(
                "job_fail_all",
                tx.send(JobResult::Error {
                    message: message.to_string()
                })
            );
        }
        if let Some(tx) = self.snapshot_ready.lock().unwrap().take() {
            capsem_core::try_send!("snapshot_ready_fail_all", tx.send(()));
        }
        // Wake any ExecDone handler parked on the deposit notifier -- it
        // will then take an empty captured and drop the stale entry.
        if let Some(active) = self.active_exec.lock().unwrap().take() {
            active.deposited.notify_waiters();
        }
        self.active_file_ops.lock().unwrap().clear();
    }
}

#[derive(Debug)]
pub(crate) enum ActiveFileOp {
    Write { path: String, data: Vec<u8> },
    Read { path: String },
}

#[derive(Debug)]
pub(crate) enum JobResult {
    Exec {
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        exit_code: i32,
        /// The guest wrote more than `MAX_EXEC_OUTPUT_BYTES`; `stdout` holds
        /// the retained prefix only.
        truncated: bool,
    },
    WriteFile {
        success: bool,
        error: Option<String>,
    },
    ReadFile {
        data: Option<Vec<u8>>,
        error: Option<String>,
    },
    LogFileBoundary {
        success: bool,
        data: Option<Vec<u8>>,
        error: Option<String>,
    },
    Error {
        message: String,
    },
}

/// Orchestrates a guest quiescence sequence around a provided async operation.
///
/// 1. Sends `PrepareSnapshot` to the guest (sync + fsfreeze).
/// 2. Waits up to `timeout` for `SnapshotReady`.
/// 3. If successful, executes the provided operation.
/// 4. Sends `Unfreeze` to the guest regardless of operation success or timeout.
#[allow(dead_code)]
pub(crate) async fn with_quiescence<F, Fut>(
    ctrl_cmd_tx: &tokio::sync::mpsc::Sender<HostToGuest>,
    job_store: &Arc<JobStore>,
    timeout: std::time::Duration,
    op: F,
) -> Result<()>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    // Prepare oneshot channel
    let (tx, rx) = tokio::sync::oneshot::channel();
    *job_store.snapshot_ready.lock().unwrap() = Some(tx);

    info!("Sending PrepareSnapshot to guest");
    ctrl_cmd_tx
        .send(HostToGuest::PrepareSnapshot)
        .await
        .map_err(|e| anyhow::anyhow!("failed to send PrepareSnapshot: {}", e))?;

    // Wait for SnapshotReady with timeout
    match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(_)) => {
            info!("Guest filesystem frozen successfully, running operation");
            let op_result = op().await;

            info!("Operation complete, sending Unfreeze to guest");
            capsem_core::try_send!(
                "ctrl_unfreeze_ok",
                ctrl_cmd_tx.send(HostToGuest::Unfreeze).await
            );
            op_result
        }
        Ok(Err(_)) => {
            // Channel closed without receiving
            warn!("SnapshotReady channel closed prematurely, aborting operation and unfreezing");
            capsem_core::try_send!(
                "ctrl_unfreeze_premature",
                ctrl_cmd_tx.send(HostToGuest::Unfreeze).await
            );
            anyhow::bail!("SnapshotReady channel closed prematurely");
        }
        Err(_) => {
            warn!("Timeout waiting for SnapshotReady, aborting operation and unfreezing");
            capsem_core::try_send!(
                "ctrl_unfreeze_timeout",
                ctrl_cmd_tx.send(HostToGuest::Unfreeze).await
            );
            anyhow::bail!("timed out waiting for SnapshotReady");
        }
    }
}

#[cfg(test)]
mod tests;
