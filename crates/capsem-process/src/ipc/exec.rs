use super::{await_exec_result, JobResult, JobStore, ProcessToService, ServiceToProcess};
use crate::job_store::ActiveExec;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

pub(super) async fn run(
    id: u64,
    command: String,
    streaming: bool,
    jobs: Arc<JobStore>,
    control: mpsc::Sender<ServiceToProcess>,
    output: mpsc::Sender<ProcessToService>,
    db: Arc<capsem_logger::DbWriter>,
) {
    let (tx, rx) = oneshot::channel();
    let installed = {
        let mut pending = jobs.jobs.lock().unwrap();
        let mut active = jobs.active_execs.lock().unwrap();
        if pending.contains_key(&id) || active.contains_key(&id) {
            false
        } else {
            let mut state = ActiveExec::new();
            state.stream = streaming.then(|| output.clone());
            if !streaming {
                state
                    .input_tx
                    .try_send(capsem_proto::ExecInputFrame::StdinEof)
                    .expect("a new exec input queue has room for EOF");
            }
            active.insert(id, state);
            drop(active);
            pending.insert(id, tx);
            true
        }
    };
    let result = if !installed {
        Err("exec id is already in use".to_string())
    } else if control.send(ServiceToProcess::Exec { id, command }).await.is_err() {
        Err("guest control channel closed".to_string())
    } else {
        // User work has no implicit duration limit. The owning IPC connection
        // cancels the guest process group when its HTTP/WebSocket caller leaves.
        // The owning IPC connection performs cancellation before dropping its
        // registrations. Watching the output queue here races that cleanup and
        // can erase the guest job before the cancellation reaches it.
        await_exec_result(rx).await
    };
    if installed {
        jobs.jobs.lock().unwrap().remove(&id);
        if let Some(active) = jobs.active_execs.lock().unwrap().remove(&id) {
            active.deposited.notify_one();
        }
        jobs.pending_acks.lock().unwrap().remove(&id);
    }
    let response = match result {
        Ok(JobResult::Exec {
            stdout,
            stderr,
            exit_code,
            truncated,
        }) => ProcessToService::ExecResult {
            id,
            stdout,
            stderr,
            exit_code,
            truncated,
        },
        other => {
            let error = match other {
                Ok(JobResult::Error { message }) => message,
                Err(error) => error,
                Ok(other) => format!("unexpected exec job result: {other:?}"),
            };
            tracing::warn!(id, %error, "exec failed");
            ProcessToService::ExecResult {
                id,
                stdout: vec![],
                stderr: error.into_bytes(),
                exit_code: -1,
                truncated: false,
            }
        }
    };
    // Preserve the existing DB-owned visibility barrier before the result.
    db.flush_after_quiescence(std::time::Duration::from_millis(50)).await;
    capsem_core::try_send!("ipc_exec_result", output.send(response).await);
}

/// Queue one stdin frame without waiting: the connection's read loop calls
/// this inline, and must stay free to read `CancelExec`. The service sends
/// within `EXEC_STDIN_WINDOW` credit, so a full queue is a protocol breach.
pub(super) fn input(id: u64, frame: capsem_proto::ExecInputFrame, jobs: &JobStore) -> Result<(), String> {
    let active = jobs.active_execs.lock().unwrap();
    let sender = match active.get(&id) {
        Some(running) => running.input_tx.clone(),
        None => return Err("exec is not running".to_string()),
    };
    drop(active);
    sender.try_send(frame).map_err(|error| match error {
        mpsc::error::TrySendError::Full(_) => "exec stdin window exceeded".to_string(),
        mpsc::error::TrySendError::Closed(_) => "exec stdin is closed".to_string(),
    })
}

pub(super) async fn cancel(id: u64, jobs: &JobStore, control: &mpsc::Sender<ServiceToProcess>) -> Result<(), String> {
    if !jobs.active_execs.lock().unwrap().contains_key(&id) {
        return Ok(());
    }
    control
        .send(ServiceToProcess::CancelExec { id })
        .await
        .map_err(|_| "guest control channel closed".to_string())
}

#[cfg(test)]
mod tests;
