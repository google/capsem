//! Worker teardown shared by reset and drop.

use super::*;

impl VirtioBlockDevice {
    pub fn with_async_notify(mut self, irq_fd: RawFd, interrupt_status: Arc<AtomicU32>, notify_fd: OwnedFd) -> Self {
        self.irq_fd = Some(irq_fd);
        self.interrupt_status = Some(interrupt_status);
        self.notify_fd = Some(notify_fd);
        self
    }

    /// Stop the ioeventfd worker, if any, and wait for it. After this the
    /// device holds no thread, no duplicated notify fd, and no ring addresses.
    pub(super) fn stop_worker(&mut self) {
        if let (Some(tx), Some(notify_fd)) = (self.control_tx.take(), self.notify_fd.as_ref()) {
            let _ = tx.send(BlockWorkerCommand::Stop);
            let _ = write_eventfd(notify_fd.as_raw_fd());
        }
        if let Some(handle) = self.worker_handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for VirtioBlockDevice {
    fn drop(&mut self) {
        self.stop_worker();
    }
}
