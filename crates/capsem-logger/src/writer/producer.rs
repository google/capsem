//! Bounded admission for the database-owned producer queue.
use super::*;

impl DbWriter {
    /// Clone the stored sender so async work can happen outside the lock.
    pub(super) fn clone_sender(&self) -> Option<WriterSender> {
        self.tx.lock().unwrap().clone()
    }

    /// Enqueue one operation, yielding while the bounded writer channel is full.
    pub async fn write(&self, op: WriteOp) {
        if let Err(error) = self.write_checked(op).await {
            warn!(error = %error, "db writer dropped write op");
        }
    }

    /// Enqueue one operation, yielding while the bounded writer channel is full.
    /// Reports a closed or missing writer instead of silently dropping the op.
    pub async fn write_checked(&self, op: WriteOp) -> Result<(), String> {
        let span = tracing::debug_span!(
            target: "capsem.db",
            DB_ENQUEUE_SPAN,
            status = tracing::field::Empty,
            queue_result = tracing::field::Empty,
        );
        let started = Instant::now();
        let Some(tx) = self.clone_sender() else {
            record_enqueue(started, "missing_sender", &span);
            return Err("db writer sender missing".to_string());
        };
        send_with_backpressure(&tx, WriterMessage::write(op))
            .await
            .inspect_err(|_| {
                record_enqueue(started, "closed", &span);
            })?;
        record_enqueue(started, "queued", &span);
        Ok(())
    }

    /// Try to enqueue without blocking. Returns false when the queue is full or closed.
    pub fn try_write(&self, op: WriteOp) -> bool {
        let span = tracing::debug_span!(
            target: "capsem.db",
            DB_ENQUEUE_SPAN,
            status = tracing::field::Empty,
            queue_result = tracing::field::Empty,
        );
        let started = Instant::now();
        let queue_result = match self.clone_sender() {
            Some(tx) => match tx.try_send(WriterMessage::write(op)) {
                Ok(()) => "queued",
                Err(mpsc::TrySendError::Full(_)) => "full",
                Err(mpsc::TrySendError::Disconnected(_)) => "closed",
            },
            None => "missing_sender",
        };
        record_enqueue(started, queue_result, &span);
        queue_result == "queued"
    }

    /// Enqueue synchronously, logging a failed admission.
    pub fn write_blocking(&self, op: WriteOp) {
        if let Err(error) = self.write_blocking_checked(op) {
            warn!(error = %error, "db writer dropped blocking write op");
        }
    }

    /// Blocking send for synchronous producer paths that must not drop
    /// security events. This deliberately avoids Tokio's `blocking_send`,
    /// which panics when called from a runtime worker. Backpressure is still
    /// honored: if the queue is full, this thread waits until the writer
    /// drains capacity instead of dropping the event.
    pub fn write_blocking_checked(&self, op: WriteOp) -> Result<(), String> {
        let span = tracing::debug_span!(
            target: "capsem.db",
            DB_ENQUEUE_SPAN,
            status = tracing::field::Empty,
            queue_result = tracing::field::Empty,
        );
        let started = Instant::now();
        let Some(tx) = self.clone_sender() else {
            record_enqueue(started, "missing_sender", &span);
            return Err("db writer sender missing".to_string());
        };
        let result = tx
            .send(WriterMessage::write(op))
            .map_err(|error| format!("db writer channel closed: {error}"));
        record_enqueue(started, if result.is_ok() { "queued" } else { "closed" }, &span);
        result
    }
}
