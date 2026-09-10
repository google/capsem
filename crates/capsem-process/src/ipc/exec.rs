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
        // User work has no implicit duration limit. Closing the streaming
        // connection releases this job's queues; the VM lifecycle owns killing
        // the command and descendants on explicit stop/delete.
        tokio::select! {
            result = await_exec_result(rx) => result,
            _ = output.closed(), if streaming => Err("exec stream consumer disconnected".to_string()),
        }
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

#[cfg(test)]
mod tests;
