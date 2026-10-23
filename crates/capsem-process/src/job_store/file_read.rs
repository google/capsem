//! The original read owns transport receipt and cancellation independently.
use super::{ActiveFileOp, JobResult, JobStore};
use capsem_core::security_engine::approval::client::ApprovalCapability;
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

#[derive(Default)]
pub(crate) struct FileReadState {
    pub(crate) received: Notify,
    pub(crate) arrived: Mutex<bool>,
    pub(crate) cancellation: CancellationToken,
    pub(crate) approvals: Option<Arc<ApprovalCapability>>,
}

impl std::fmt::Debug for FileReadState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("FileReadState").finish_non_exhaustive()
    }
}

impl FileReadState {
    pub(crate) fn cancel(&self) {
        let _phase = self.arrived.lock().unwrap();
        self.cancellation.cancel();
        self.received.notify_one();
    }
}

impl JobStore {
    pub(crate) fn retain_file_bytes(
        &self,
        data_bytes: usize,
        context_bytes: usize,
    ) -> Option<tokio::sync::OwnedSemaphorePermit> {
        // Bound original buffers plus transient typed/UTF-8/serialized preview
        // copies before any ledger or policy await. Normal 10 MiB transfers
        // still fit the 16 MiB owner when uncontended; overload never queues.
        let preview = data_bytes.min(crate::ipc::file_boundary::FILE_SECURITY_CONTENT_PREVIEW_MAX);
        let bytes = data_bytes
            .saturating_add(preview.saturating_mul(32))
            .saturating_add(context_bytes.saturating_mul(8))
            .saturating_add(capsem_proto::APPROVAL_CONTEXT_BYTES);
        Arc::clone(&self.file_memory)
            .try_acquire_many_owned(u32::try_from(bytes).ok()?)
            .ok()
    }

    pub(crate) fn cancel_file_reads(&self) {
        for active in self.active_file_ops.lock().unwrap().values() {
            if let ActiveFileOp::Read { state, .. } = active {
                state.cancel();
            }
        }
    }

    pub(crate) fn take_file_read_reply(
        &self,
        id: u64,
        state: &Arc<FileReadState>,
    ) -> Option<oneshot::Sender<JobResult>> {
        let mut jobs = self.jobs.lock().unwrap();
        let active = self.active_file_ops.lock().unwrap();
        if matches!(active.get(&id), Some(ActiveFileOp::Read { state: current, .. }) if Arc::ptr_eq(current, state)) {
            jobs.remove(&id)
        } else {
            None
        }
    }

    pub(crate) fn finish_file_read(&self, id: u64, state: &Arc<FileReadState>) {
        let mut jobs = self.jobs.lock().unwrap();
        let mut active = self.active_file_ops.lock().unwrap();
        if matches!(active.get(&id), Some(ActiveFileOp::Read { state: current, .. }) if Arc::ptr_eq(current, state)) {
            active.remove(&id);
            drop(active);
            self.pending_acks.lock().unwrap().remove(&id);
            jobs.remove(&id);
        }
    }

    pub(crate) fn deliver_file_read(
        &self,
        id: u64,
        state: &Arc<FileReadState>,
        sender: oneshot::Sender<JobResult>,
        result: JobResult,
    ) {
        let active = self.active_file_ops.lock().unwrap();
        let _phase = state.arrived.lock().unwrap();
        if matches!(active.get(&id), Some(ActiveFileOp::Read { state: current, .. }) if Arc::ptr_eq(current, state))
            && !state.cancellation.is_cancelled()
            && !sender.is_closed()
        {
            let _ = sender.send(result); // channel-closed-ok: caller cancellation must never log file bytes.
        }
    }
}
