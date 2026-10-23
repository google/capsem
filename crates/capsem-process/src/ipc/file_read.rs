//! Original read jobs, owned and bounded by their IPC connection.
use super::{McpRuntime, ProcessToService, ServiceToProcess, GUEST_PAYLOAD_MAX_RETRIES, GUEST_PAYLOAD_TIMEOUT};
use crate::job_store::{ActiveFileOp, FileReadState, JobResult, JobStore};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Semaphore};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub(super) struct ReadFiles {
    jobs: Arc<JobStore>,
    control: mpsc::Sender<ServiceToProcess>,
    output: mpsc::Sender<ProcessToService>,
    approvals: Option<Arc<capsem_core::security_engine::approval::client::ApprovalCapability>>,
    capacity: Arc<Semaphore>,
    cancellation: CancellationToken,
}

struct ReadLease {
    jobs: Arc<JobStore>,
    state: Arc<FileReadState>,
    id: u64,
}

impl Drop for ReadLease {
    fn drop(&mut self) {
        self.state.cancel();
        self.jobs.finish_file_read(self.id, &self.state);
    }
}

impl ReadFiles {
    pub(super) fn new(
        jobs: Arc<JobStore>,
        control: mpsc::Sender<ServiceToProcess>,
        output: mpsc::Sender<ProcessToService>,
        runtime: &McpRuntime,
        capacity: Arc<Semaphore>,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            jobs,
            control,
            output,
            approvals: runtime.approvals.clone(),
            capacity,
            cancellation,
        }
    }

    pub(super) async fn spawn(&self, calls: &mut JoinSet<()>, id: u64, path: String) {
        let permit = match Arc::clone(&self.capacity).try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                self.reply(id, Err("file read capacity exceeded".into())).await;
                return;
            }
        };
        let state = Arc::new(FileReadState {
            approvals: self.approvals.clone(),
            ..Default::default()
        });
        let (sender, receiver) = oneshot::channel();
        let registered = {
            let mut jobs = self.jobs.jobs.lock().unwrap();
            let mut active = self.jobs.active_file_ops.lock().unwrap();
            if jobs.contains_key(&id) || active.contains_key(&id) {
                false
            } else {
                jobs.insert(id, sender);
                drop(jobs);
                active.insert(
                    id,
                    ActiveFileOp::Read {
                        path: path.clone(),
                        state: Arc::clone(&state),
                    },
                );
                true
            }
        };
        if !registered {
            self.reply(id, Err("file read identity is already active".into())).await;
            return;
        }
        let lease = ReadLease {
            jobs: Arc::clone(&self.jobs),
            state: Arc::clone(&state),
            id,
        };
        let reader = self.clone();
        calls.spawn(async move {
            let _permit = permit;
            let _lease = lease;
            let result = tokio::select! {
                biased;
                _ = reader.cancellation.cancelled() => return,
                _ = state.cancellation.cancelled() => Err("file read cancelled".into()),
                result = reader.wait(id, path, &state, receiver) => result,
            };
            reader.reply(id, result).await;
        });
    }

    async fn reply(&self, id: u64, result: Result<JobResult, String>) {
        let (data, error) = match result {
            Ok(JobResult::ReadFile { data, error }) => (data, error),
            Ok(JobResult::Error { message }) => (None, Some(message)),
            Ok(_) => (None, Some("unexpected file read completion".into())),
            Err(error) => (None, Some(error)),
        };
        tokio::select! {
            biased;
            _ = self.cancellation.cancelled() => {},
            result = self.output.send(ProcessToService::ReadFileResult { id, data, error }) => {
                let _ = result; // channel-closed-ok: caller cancellation must never log file bytes.
            }
        }
    }

    async fn wait(
        &self,
        id: u64,
        path: String,
        state: &FileReadState,
        mut result: oneshot::Receiver<JobResult>,
    ) -> Result<JobResult, String> {
        self.control
            .send(ServiceToProcess::ReadFile { id, path: path.clone() })
            .await
            .map_err(|_| "file read control closed")?;
        let mut retries = 0;
        loop {
            tokio::select! {
                biased;
                // This is a transport receipt, not an approval timer reset.
                // The original result still waits on its caller-owned lease.
                _ = state.received.notified() => return result.await.map_err(|_| "file read callback closed".into()),
                outcome = tokio::time::timeout(GUEST_PAYLOAD_TIMEOUT, &mut result) => match outcome {
                    Ok(value) => return value.map_err(|_| "file read callback closed".into()),
                    Err(_) => {
                        if retries >= GUEST_PAYLOAD_MAX_RETRIES { return Err("FileRead watchdog exhausted retries".into()); }
                        retries += 1;
                        self.control.send(ServiceToProcess::ReadFile { id, path: path.clone() }).await.map_err(|_| "file read control closed")?;
                    }
                }
            }
        }
    }
}
