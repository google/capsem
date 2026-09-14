//! Credits cover setup, active streams, queued terminal reports and replay/ACK.
use super::Queued;
use capsem_proto::router::{CloseReport, FlowKey};
use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub(crate) struct Reports {
    credits: Arc<Semaphore>,
    pending: Mutex<HashMap<FlowKey, Queued>>,
    failed: std::sync::atomic::AtomicBool,
}

impl Default for Reports {
    fn default() -> Self {
        Self {
            credits: Arc::new(Semaphore::new(64)),
            pending: Mutex::new(HashMap::new()),
            failed: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

impl Reports {
    pub(crate) fn reserve(&self) -> io::Result<Arc<OwnedSemaphorePermit>> {
        self.credits
            .clone()
            .try_acquire_owned()
            .map(Arc::new)
            .map_err(|_| io::Error::other("network close report quota exhausted"))
    }

    pub(super) fn park(
        &self,
        flow: FlowKey,
        report: CloseReport,
        credit: Arc<OwnedSemaphorePermit>,
    ) -> io::Result<Queued> {
        let mut pending = self.pending.lock().unwrap();
        let std::collections::hash_map::Entry::Vacant(entry) = pending.entry(flow) else {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "duplicate network close report",
            ));
        };
        let queued = Queued {
            message: capsem_proto::GuestToHost::PortClosed { flow, report },
            credit: Some(credit),
            acknowledged: Some(Arc::new(tokio::sync::Notify::new())),
        };
        entry.insert(queued.clone());
        drop(pending);
        Ok(queued)
    }

    pub(crate) fn acknowledge(&self, flow: FlowKey) {
        let queued = self.pending.lock().unwrap().remove(&flow);
        if let Some(queued) = queued {
            queued.acknowledged.unwrap().notify_one();
        }
    }

    pub(crate) fn fail(&self) {
        self.failed.store(true, std::sync::atomic::Ordering::Release);
    }

    pub(super) fn take_failure(&self) -> bool {
        self.failed.swap(false, std::sync::atomic::Ordering::AcqRel)
    }

    pub(super) fn snapshot(&self) -> Vec<Queued> {
        self.pending.lock().unwrap().values().cloned().collect()
    }
}
