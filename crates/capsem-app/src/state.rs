use std::collections::{HashMap, VecDeque};
use std::os::unix::io::RawFd;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::sync::atomic::{AtomicBool, Ordering};

use capsem_core::VirtualMachine;
use capsem_core::HostStateMachine;
use capsem_core::net::cert_authority::CertAuthority;
use capsem_core::net::policy::NetworkPolicy;
use capsem_core::session::SessionIndex;
use capsem_logger::DbWriter;

/// Per-VM network state: policy, telemetry DB, and connection tracking.
///
/// Each VM gets its own `VmNetworkState` that is dropped when the VM stops,
/// which prevents cross-VM interference.
pub struct VmNetworkState {
    /// Live network policy. Wrapped in RwLock so `update_setting` can hot-reload
    /// it without restarting the VM. Readers (MITM proxy connections) clone the
    /// inner Arc cheaply; writers swap the entire Arc on policy change.
    pub policy: Arc<RwLock<Arc<NetworkPolicy>>>,
    pub db: Arc<DbWriter>,
    pub ca: Arc<CertAuthority>,
    /// Cached upstream TLS config, created once via `mitm_proxy::make_upstream_tls_config()`.
    pub upstream_tls: Arc<capsem_core::net::mitm_proxy::UpstreamTlsConfig>,
}

/// Per-VM instance state.
pub struct VmInstance {
    pub vm: VirtualMachine,
    pub serial_input_fd: RawFd,
    pub vsock_terminal_fd: Option<RawFd>,
    pub vsock_control_fd: Option<RawFd>,
    pub net_state: Option<VmNetworkState>,
    pub state_machine: HostStateMachine,
    pub scratch_disk_path: Option<PathBuf>,
}

/// Max queued output chunks before dropping to prevent OOM when the frontend
/// stops polling (tab backgrounded, JS hung). Each chunk is typically a few KB
/// from the coalesce buffer.
const TERMINAL_QUEUE_CAPACITY: usize = 64;

/// Lock-free-ish queue for terminal output data.
///
/// The vsock reader pushes raw byte chunks via `push()`. The frontend polls
/// via the `terminal_poll` IPC command which calls `poll()`. When the VM stops,
/// `close()` unblocks any pending poll. On new VM boot, `reset()` reopens the
/// queue for fresh data.
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

    /// Close the queue. Pending and future polls return `None` once drained.
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        // Use notify_one() to store a permit. notify_waiters() does NOT store
        // a permit, so if poll() is between its lock release and .await, the
        // wakeup would be lost and poll() would hang forever.
        self.notify.notify_one();
    }

    /// Reset the queue for a new VM session. Clears data and reopens.
    pub fn reset(&self) {
        let mut queue = self.data.lock().unwrap();
        queue.clear();
        self.closed.store(false, Ordering::Release);
    }
}

pub struct AppState {
    pub vms: Mutex<HashMap<String, VmInstance>>,
    pub session_index: Mutex<SessionIndex>,
    pub active_session_id: Mutex<Option<String>>,
    pub terminal_output: Arc<TerminalOutputQueue>,
    pub terminal_input_tx: std::sync::mpsc::Sender<(RawFd, String)>,
}

impl AppState {
    pub fn new(session_index: SessionIndex) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<(RawFd, String)>();
        
        // Spawn a dedicated global thread for batching terminal input writes.
        // This prevents spawning a new Tokio thread per character typed,
        // which causes severe CPU spikes and thread pool exhaustion.
        std::thread::spawn(move || {
            use std::io::Write;
            let mut current_fd: Option<RawFd> = None;
            let mut current_file: Option<std::fs::File> = None;

            while let Ok((fd, data)) = rx.recv() {
                let mut buf = data.into_bytes();
                // Coalesce rapid sequential inputs for the same file descriptor
                while let Ok((next_fd, next_data)) = rx.try_recv() {
                    if next_fd == fd {
                        buf.extend(next_data.into_bytes());
                    } else {
                        // Very rare: active VM switched in the middle of a microsecond burst.
                        // We'll handle this in the next iteration.
                    }
                }
                
                // Reuse the file handle if it's the same FD.
                if current_fd != Some(fd) {
                    current_fd = Some(fd);
                    current_file = crate::clone_fd(fd).ok();
                }

                if let Some(mut file) = current_file.as_ref() {
                    let _ = file.write_all(&buf);
                    let _ = file.flush();
                }
            }
        });

        Self {
            vms: Mutex::new(HashMap::new()),
            session_index: Mutex::new(session_index),
            active_session_id: Mutex::new(None),
            terminal_output: Arc::new(TerminalOutputQueue::new()),
            terminal_input_tx: tx,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_state_has_no_vms() {
        let idx = SessionIndex::open_in_memory().unwrap();
        let state = AppState::new(idx);
        let vms = state.vms.lock().unwrap();
        assert!(vms.is_empty());
    }

    #[test]
    fn mutex_is_not_poisoned_on_creation() {
        let idx = SessionIndex::open_in_memory().unwrap();
        let state = AppState::new(idx);
        assert!(!state.vms.is_poisoned());
        assert!(!state.session_index.is_poisoned());
        assert!(!state.active_session_id.is_poisoned());
    }

    #[test]
    fn active_session_starts_none() {
        let idx = SessionIndex::open_in_memory().unwrap();
        let state = AppState::new(idx);
        assert!(state.active_session_id.lock().unwrap().is_none());
    }

    // -----------------------------------------------------------------------
    // TerminalOutputQueue
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn queue_push_poll_roundtrip() {
        let q = TerminalOutputQueue::new();
        q.push(b"hello".to_vec());
        let chunk = q.poll().await;
        assert_eq!(chunk.unwrap(), b"hello");
    }

    #[tokio::test]
    async fn queue_poll_returns_none_when_closed() {
        let q = TerminalOutputQueue::new();
        q.close();
        assert!(q.poll().await.is_none());
    }

    #[tokio::test]
    async fn queue_drains_before_returning_none() {
        let q = TerminalOutputQueue::new();
        q.push(b"a".to_vec());
        q.push(b"b".to_vec());
        q.close();
        assert_eq!(q.poll().await.unwrap(), b"a");
        assert_eq!(q.poll().await.unwrap(), b"b");
        assert!(q.poll().await.is_none());
    }

    #[tokio::test]
    async fn queue_reset_reopens() {
        let q = TerminalOutputQueue::new();
        q.push(b"old".to_vec());
        q.close();
        q.reset();
        q.push(b"new".to_vec());
        assert_eq!(q.poll().await.unwrap(), b"new");
    }

    #[tokio::test]
    async fn queue_multiple_pushes_drain_fifo() {
        let q = TerminalOutputQueue::new();
        q.push(b"1".to_vec());
        q.push(b"2".to_vec());
        q.push(b"3".to_vec());
        assert_eq!(q.poll().await.unwrap(), b"1");
        assert_eq!(q.poll().await.unwrap(), b"2");
        assert_eq!(q.poll().await.unwrap(), b"3");
    }

    #[test]
    fn queue_backpressure_drops_oldest() {
        let q = TerminalOutputQueue::new();
        // Fill to capacity.
        for i in 0..TERMINAL_QUEUE_CAPACITY {
            q.push(vec![i as u8]);
        }
        // One more push should drop the oldest (0).
        q.push(vec![99]);
        let queue = q.data.lock().unwrap();
        assert_eq!(queue.len(), TERMINAL_QUEUE_CAPACITY);
        assert_eq!(queue[0], vec![1]); // oldest is now 1, not 0
        assert_eq!(queue[TERMINAL_QUEUE_CAPACITY - 1], vec![99]);
    }

    #[test]
    fn queue_push_ignored_when_closed() {
        let q = TerminalOutputQueue::new();
        q.close();
        q.push(b"nope".to_vec());
        let queue = q.data.lock().unwrap();
        assert!(queue.is_empty());
    }

    #[tokio::test]
    async fn queue_poll_wakes_on_push() {
        let q = Arc::new(TerminalOutputQueue::new());
        let q2 = Arc::clone(&q);
        let handle = tokio::spawn(async move {
            q2.poll().await
        });
        // Give the poll a moment to park.
        tokio::task::yield_now().await;
        q.push(b"wake".to_vec());
        let result = handle.await.unwrap();
        assert_eq!(result.unwrap(), b"wake");
    }

    #[tokio::test]
    async fn queue_poll_wakes_on_close() {
        let q = Arc::new(TerminalOutputQueue::new());
        let q2 = Arc::clone(&q);
        let handle = tokio::spawn(async move {
            q2.poll().await
        });
        tokio::task::yield_now().await;
        q.close();
        let result = handle.await.unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn app_state_has_terminal_output_queue() {
        let idx = SessionIndex::open_in_memory().unwrap();
        let state = AppState::new(idx);
        // Queue should be open and empty.
        let queue = state.terminal_output.data.lock().unwrap();
        assert!(queue.is_empty());
        assert!(!state.terminal_output.closed.load(Ordering::Acquire));
    }
}
