use std::collections::VecDeque;
use std::sync::{Mutex, atomic::{AtomicBool, Ordering}};

/// Maximum bytes to coalesce in a single batch (1MB).
const DEFAULT_MAX_COALESCE_BYTES: usize = 1024 * 1024;

/// Max queued output chunks before dropping to prevent OOM when the consumer
/// stops polling. Each chunk is typically a few KB from the coalesce buffer.
const TERMINAL_QUEUE_CAPACITY: usize = 1024;

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
            tracing::warn!("TerminalOutputQueue capacity reached, dropped oldest chunk");
        }
        queue.push_back(bytes);
        drop(queue);
        self.notify.notify_one();
    }

    /// Async poll for the next chunk. Aggressively coalesces up to ~1MB of data
    /// from the queue into a single byte vector to minimize IPC/syscall overhead.
    /// Returns `None` when the queue is closed and drained.
    pub async fn poll(&self) -> Option<Vec<u8>> {
        loop {
            // Try to pop a chunk under the lock.
            {
                let mut queue = self.data.lock().unwrap();
                if let Some(mut coalesced) = queue.pop_front() {
                    let mut count = 1;
                    
                    // Keep draining the queue into our coalesced buffer until we hit 1MB
                    while coalesced.len() < DEFAULT_MAX_COALESCE_BYTES {
                        if let Some(next) = queue.front() {
                            if coalesced.len() + next.len() > DEFAULT_MAX_COALESCE_BYTES {
                                break; // Adding this chunk would exceed our max batch size
                            }
                            coalesced.extend_from_slice(&queue.pop_front().unwrap());
                            count += 1;
                        } else {
                            break; // Queue empty
                        }
                    }

                    if count > 1 {
                        tracing::debug!(
                            "Sender side coalesced {} terminal chunks into {} bytes before IPC transfer",
                            count,
                            coalesced.len()
                        );
                    }
                    
                    return Some(coalesced);
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
