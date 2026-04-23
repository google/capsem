use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use anyhow::Result;
use capsem_proto::HostToGuest;
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
    /// Channel for snapshot ready signal.
    pub(crate) snapshot_ready: Mutex<Option<oneshot::Sender<()>>>,
}

/// State for an in-flight exec. `deposited` is notified once by the
/// EXEC-port reader thread after it has written `captured` under the
/// active_exec lock; ExecDone uses it to distinguish "empty stdout" from
/// "deposit still in flight" without sleeping unconditionally.
pub(crate) struct ActiveExec {
    pub(crate) id: u64,
    pub(crate) captured: Vec<u8>,
    pub(crate) deposited: Arc<Notify>,
}

impl ActiveExec {
    pub(crate) fn new(id: u64) -> Self {
        Self {
            id,
            captured: Vec::new(),
            deposited: Arc::new(Notify::new()),
        }
    }
}

impl JobStore {
    pub(crate) fn new() -> Self {
        Self {
            jobs: Mutex::new(HashMap::new()),
            active_exec: Mutex::new(None),
            snapshot_ready: Mutex::new(None),
        }
    }

    /// Drain every pending job and oneshot, answering each with an Error.
    /// Called when the control-channel reader has died so callers waiting on
    /// the oneshots (see `ipc.rs` Exec/WriteFile/ReadFile handlers) get a
    /// prompt failure instead of a 30s IPC timeout.
    pub(crate) fn fail_all(&self, message: &str) {
        let pending: Vec<_> = self.jobs.lock().unwrap().drain().collect();
        for (_id, tx) in pending {
            let _ = tx.send(JobResult::Error { message: message.to_string() });
        }
        if let Some(tx) = self.snapshot_ready.lock().unwrap().take() {
            let _ = tx.send(());
        }
        // Wake any ExecDone handler parked on the deposit notifier -- it
        // will then take an empty captured and drop the stale entry.
        if let Some(active) = self.active_exec.lock().unwrap().take() {
            active.deposited.notify_waiters();
        }
    }
}

#[cfg_attr(test, derive(Debug))]
pub(crate) enum JobResult {
    Exec { stdout: Vec<u8>, stderr: Vec<u8>, exit_code: i32 },
    WriteFile { success: bool, error: Option<String> },
    ReadFile { data: Option<Vec<u8>>, error: Option<String> },
    Error { message: String },
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
            let _ = ctrl_cmd_tx.send(HostToGuest::Unfreeze).await;
            op_result
        }
        Ok(Err(_)) => {
            // Channel closed without receiving
            warn!("SnapshotReady channel closed prematurely, aborting operation and unfreezing");
            let _ = ctrl_cmd_tx.send(HostToGuest::Unfreeze).await;
            anyhow::bail!("SnapshotReady channel closed prematurely");
        }
        Err(_) => {
            warn!("Timeout waiting for SnapshotReady, aborting operation and unfreezing");
            let _ = ctrl_cmd_tx.send(HostToGuest::Unfreeze).await;
            anyhow::bail!("timed out waiting for SnapshotReady");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // JobStore
    // -----------------------------------------------------------------------

    #[test]
    fn job_store_insert_and_remove() {
        let store = JobStore::new();
        let (tx, _rx) = oneshot::channel::<JobResult>();
        store.jobs.lock().unwrap().insert(1, tx);
        assert!(store.jobs.lock().unwrap().contains_key(&1));
        let removed = store.jobs.lock().unwrap().remove(&1);
        assert!(removed.is_some());
        assert!(!store.jobs.lock().unwrap().contains_key(&1));
    }

    #[test]
    fn job_store_missing_id_returns_none() {
        let store = JobStore::new();
        let removed = store.jobs.lock().unwrap().remove(&999);
        assert!(removed.is_none());
    }

    #[test]
    fn job_store_concurrent_ids_unique() {
        let store = JobStore::new();
        for i in 0..100 {
            let (tx, _rx) = oneshot::channel::<JobResult>();
            store.jobs.lock().unwrap().insert(i, tx);
        }
        assert_eq!(store.jobs.lock().unwrap().len(), 100);
    }

    #[test]
    fn job_store_active_exec_set_and_clear() {
        let store = JobStore::new();
        assert!(store.active_exec.lock().unwrap().is_none());

        *store.active_exec.lock().unwrap() = Some(ActiveExec::new(42));
        {
            let guard = store.active_exec.lock().unwrap();
            let active = guard.as_ref().unwrap();
            assert_eq!(active.id, 42);
            assert!(active.captured.is_empty());
        }

        *store.active_exec.lock().unwrap() = None;
        assert!(store.active_exec.lock().unwrap().is_none());
    }

    #[test]
    fn job_store_active_exec_captures_data() {
        let store = JobStore::new();
        *store.active_exec.lock().unwrap() = Some(ActiveExec::new(1));
        if let Some(ref mut active) = *store.active_exec.lock().unwrap() {
            active.captured.extend_from_slice(b"hello ");
            active.captured.extend_from_slice(b"world");
        }
        let captured = store.active_exec.lock().unwrap().as_ref().unwrap().captured.clone();
        assert_eq!(captured, b"hello world");
    }

    #[test]
    fn job_store_overwrite_same_id() {
        let store = JobStore::new();
        let (tx1, _rx1) = oneshot::channel::<JobResult>();
        let (tx2, _rx2) = oneshot::channel::<JobResult>();
        store.jobs.lock().unwrap().insert(1, tx1);
        // Overwriting drops the old sender
        store.jobs.lock().unwrap().insert(1, tx2);
        assert_eq!(store.jobs.lock().unwrap().len(), 1);
    }

    // -----------------------------------------------------------------------
    // JobResult variants
    // -----------------------------------------------------------------------

    #[test]
    fn job_result_exec_fields() {
        let r = JobResult::Exec {
            stdout: b"output".to_vec(),
            stderr: b"err".to_vec(),
            exit_code: 0,
        };
        match r {
            JobResult::Exec { stdout, stderr, exit_code } => {
                assert_eq!(stdout, b"output");
                assert_eq!(stderr, b"err");
                assert_eq!(exit_code, 0);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn job_result_exec_nonzero_exit() {
        let r = JobResult::Exec {
            stdout: vec![],
            stderr: b"command not found".to_vec(),
            exit_code: 127,
        };
        match r {
            JobResult::Exec { exit_code, .. } => assert_eq!(exit_code, 127),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn job_result_write_file_success() {
        let r = JobResult::WriteFile { success: true, error: None };
        match r {
            JobResult::WriteFile { success, error } => {
                assert!(success);
                assert!(error.is_none());
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn job_result_write_file_error() {
        let r = JobResult::WriteFile { success: false, error: Some("permission denied".into()) };
        match r {
            JobResult::WriteFile { success, error } => {
                assert!(!success);
                assert_eq!(error.unwrap(), "permission denied");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn job_result_read_file_with_data() {
        let r = JobResult::ReadFile { data: Some(b"contents".to_vec()), error: None };
        match r {
            JobResult::ReadFile { data, error } => {
                assert_eq!(data.unwrap(), b"contents");
                assert!(error.is_none());
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn job_result_read_file_not_found() {
        let r = JobResult::ReadFile { data: None, error: Some("not found".into()) };
        match r {
            JobResult::ReadFile { data, error } => {
                assert!(data.is_none());
                assert_eq!(error.unwrap(), "not found");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn job_result_error() {
        let r = JobResult::Error { message: "internal failure".into() };
        match r {
            JobResult::Error { message } => assert_eq!(message, "internal failure"),
            _ => panic!("wrong variant"),
        }
    }

    // -----------------------------------------------------------------------
    // fail_all drains every pending oneshot with an Error
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn fail_all_resolves_every_pending_oneshot() {
        let job_store = Arc::new(JobStore::new());
        // Register three pending jobs + a snapshot_ready waiter.
        let (tx1, rx1) = oneshot::channel::<JobResult>();
        let (tx2, rx2) = oneshot::channel::<JobResult>();
        let (tx3, rx3) = oneshot::channel::<JobResult>();
        let (snap_tx, snap_rx) = oneshot::channel::<()>();
        {
            let mut jobs = job_store.jobs.lock().unwrap();
            jobs.insert(1, tx1);
            jobs.insert(2, tx2);
            jobs.insert(3, tx3);
        }
        *job_store.snapshot_ready.lock().unwrap() = Some(snap_tx);
        {
            let mut active = ActiveExec::new(1);
            active.captured = b"buffered".to_vec();
            *job_store.active_exec.lock().unwrap() = Some(active);
        }

        // Regression guard: this is the crucial behavior -- callers awaiting
        // these oneshots must see an immediate result, not hang forever and
        // let the parent IPC call time out at 30s.
        job_store.fail_all("control channel closed: decode error");

        for rx in [rx1, rx2, rx3] {
            match rx.await {
                Ok(JobResult::Error { message }) => {
                    assert!(message.contains("control channel closed"));
                }
                other => panic!("expected JobResult::Error, got {other:?}"),
            }
        }
        assert!(snap_rx.await.is_ok(), "snapshot_ready waiter must be resolved");
        assert!(job_store.active_exec.lock().unwrap().is_none());
        assert!(job_store.jobs.lock().unwrap().is_empty());
    }

    // -----------------------------------------------------------------------
    // Job completion via oneshot (integration-unit)
    // -----------------------------------------------------------------------

    #[test]
    fn job_oneshot_send_receive() {
        let (tx, rx) = oneshot::channel::<JobResult>();
        tx.send(JobResult::Exec {
            stdout: b"hello".to_vec(),
            stderr: vec![],
            exit_code: 0,
        }).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(rx).unwrap();
        match result {
            JobResult::Exec { stdout, exit_code, .. } => {
                assert_eq!(stdout, b"hello");
                assert_eq!(exit_code, 0);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn job_oneshot_dropped_sender() {
        let (tx, rx) = oneshot::channel::<JobResult>();
        drop(tx); // Simulate client disconnect

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(rx);
        assert!(result.is_err()); // RecvError
    }

    #[tokio::test]
    async fn quiescence_timeout_fires() {
        let job_store = Arc::new(JobStore::new());
        let (tx, mut _rx) = tokio::sync::mpsc::channel::<HostToGuest>(16);

        // 1. Never send SnapshotReady
        let start = std::time::Instant::now();
        let result = with_quiescence(&tx, &job_store, std::time::Duration::from_millis(100), || async {
            Ok(())
        }).await;

        let elapsed = start.elapsed();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("timed out"));
        assert!(elapsed.as_millis() >= 100);
    }

    #[tokio::test]
    async fn quiescence_success_runs_operation() {
        let job_store = Arc::new(JobStore::new());
        let (tx, mut _rx) = tokio::sync::mpsc::channel::<HostToGuest>(16);

        // Simulate the guest sending SnapshotReady
        {
            let js = Arc::clone(&job_store);
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                if let Some(sender) = js.snapshot_ready.lock().unwrap().take() {
                    let _ = sender.send(());
                }
            });
        }

        let executed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let executed_clone = Arc::clone(&executed);
        let result = with_quiescence(&tx, &job_store, std::time::Duration::from_secs(5), || {
            let e = executed_clone;
            async move {
                e.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
        }).await;

        assert!(result.is_ok());
        assert!(executed.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn quiescence_channel_closed_returns_error() {
        let job_store = Arc::new(JobStore::new());
        let (tx, mut _rx) = tokio::sync::mpsc::channel::<HostToGuest>(16);

        // Drop the sender without sending, simulating channel close
        {
            let js = Arc::clone(&job_store);
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                // Take and drop the sender without sending
                let _ = js.snapshot_ready.lock().unwrap().take();
            });
        }

        let result = with_quiescence(&tx, &job_store, std::time::Duration::from_secs(5), || async {
            Ok(())
        }).await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("closed prematurely"));
    }
}
