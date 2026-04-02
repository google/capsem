use std::collections::VecDeque;
use std::sync::{Mutex, atomic::{AtomicBool, Ordering}};

/// Max queued output chunks before dropping to prevent OOM when the consumer
/// stops polling. Each chunk is typically a few KB from the coalesce buffer.
const TERMINAL_QUEUE_CAPACITY: usize = 64;

/// Lock-free-ish queue for terminal output data.
///
/// The vsock reader pushes raw byte chunks via `push()`. The consumer (CLI or 
/// frontend) polls via `poll()`. When the VM stops, `close()` unblocks any 
/// pending poll. On new VM boot, `reset()` reopens the queue for fresh data.
pub struct TerminalOutputQueue {
    data: Mutex<VecDeque<Vec<u8>>>,
    notify: tokio::sync::Notify,
    closed: AtomicBool,
}

impl TerminalOutputQueue {
    pub fn new() -> Self {
        Self {
            data: Mutex::new(VecDeque::new()),
            notify: tokio::sync::Notify::new(),
            closed: AtomicBool::new(false),
        }
    }

    /// Push a chunk of terminal output. Drops the chunk if the queue is at
    /// capacity (backpressure) or closed.
    pub fn push(&self, bytes: Vec<u8>) {
        if self.closed.load(Ordering::Acquire) {
            return;
        }
        let mut queue = self.data.lock().unwrap();
        if queue.len() >= TERMINAL_QUEUE_CAPACITY {
            // Backpressure: drop oldest to make room.
            queue.pop_front();
        }
        queue.push_back(bytes);
        drop(queue);
        self.notify.notify_one();
    }

    /// Async poll for the next chunk. Returns `None` when the queue is closed
    /// and drained.
    pub async fn poll(&self) -> Option<Vec<u8>> {
        loop {
            // Try to pop a chunk under the lock.
            {
                let mut queue = self.data.lock().unwrap();
                if let Some(chunk) = queue.pop_front() {
                    return Some(chunk);
                }
                // Queue is empty. If closed, we're done.
                if self.closed.load(Ordering::Acquire) {
                    return None;
                }
            }
            // Wait for a notification (push or close).
            self.notify.notified().await;
        }
    }

    /// Mark the queue as closed. Wakes any pending `poll()` which will return
    /// `None` once the remaining data is drained.
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.notify.notify_one();
    }

    /// Reset the queue for a new VM session. Clears data and reopens.
    pub fn reset(&self) {
        let mut queue = self.data.lock().unwrap();
        queue.clear();
        self.closed.store(false, Ordering::Release);
    }
}

impl Default for TerminalOutputQueue {
    fn default() -> Self {
        Self::new()
    }
}
