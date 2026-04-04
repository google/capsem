use std::collections::VecDeque;
use tokio_unix_ipc::Receiver;
use capsem_proto::ipc::ProcessToService;

/// Default maximum bytes to coalesce in a single batch (1MB).
pub const DEFAULT_MAX_COALESCE_BYTES: usize = 1024 * 1024;

/// A wrapper around a tokio_unix_ipc Receiver that safely coalesces consecutive
/// stream chunks (like `TerminalOutput`) without losing other interleaved messages.
pub struct StreamCoalescer {
    rx: Receiver<ProcessToService>,
    pushback: VecDeque<ProcessToService>,
    max_bytes: usize,
}

impl StreamCoalescer {
    /// Create a new coalescer with the default 1MB max buffer size.
    pub fn new(rx: Receiver<ProcessToService>) -> Self {
        Self::with_max_bytes(rx, DEFAULT_MAX_COALESCE_BYTES)
    }

    /// Create a new coalescer with a custom max buffer size.
    pub fn with_max_bytes(rx: Receiver<ProcessToService>, max_bytes: usize) -> Self {
        Self {
            rx,
            pushback: VecDeque::new(),
            max_bytes,
        }
    }

    /// Receive the next message, aggressively coalescing consecutive stream chunks.
    pub async fn recv(&mut self) -> Option<ProcessToService> {
        // Yield pushed-back messages first
        if let Some(msg) = self.pushback.pop_front() {
            return Some(msg);
        }

        // Wait for the next message
        let first = self.rx.recv().await.ok()?;

        let mut coalesced = match first {
            ProcessToService::TerminalOutput { data } => data,
            other => return Some(other),
        };

        let mut count = 1;
        let start_len = coalesced.len();

        // Aggressively coalesce any immediately available stream chunks
        loop {
            tokio::select! {
                biased;
                res = self.rx.recv() => {
                    match res {
                        Ok(ProcessToService::TerminalOutput { data }) => {
                            count += 1;
                            coalesced.extend_from_slice(&data);
                            if coalesced.len() >= self.max_bytes {
                                break; // Prevent unbounded memory growth
                            }
                        }
                        Ok(other) => {
                            // We hit a non-stream message. Push it back for the next recv()
                            self.pushback.push_back(other);
                            break;
                        }
                        Err(_) => break, // Socket closed
                    }
                }
                _ = async {} => {
                    break; // No more immediate messages on the wire
                }
            }
        }

        if count > 1 {
            tracing::debug!(
                "coalesced {} stream chunks into {} bytes (was {})",
                count,
                coalesced.len(),
                start_len
            );
        }

        Some(ProcessToService::TerminalOutput { data: coalesced })
    }
}
