use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

/// Maximum bytes kept in the replay ring buffer. 64 KiB covers typical
/// login banners, MOTD, and a few screenfuls of output -- enough for a
/// freshly attached terminal stream to see what the shell printed before it
/// arrived without unbounded memory growth.
pub const REPLAY_BUFFER_SIZE: usize = 64 * 1024;

/// Fan-out relay for PTY output. Live subscribers receive new bytes via the
/// broadcast channel; newly-subscribing clients additionally get the last
/// `REPLAY_BUFFER_SIZE` bytes of output so they see the shell's startup
/// banner even if the shell printed it before the stream attached.
///
/// Thread safety: `publish` and `subscribe` both take the same Mutex, which
/// serializes buffer append + broadcast send with buffer snapshot + broadcast
/// subscribe. This avoids a race where a byte is either duplicated (seen in
/// both replay and live stream) or lost (missed by both).
pub struct TerminalRelay {
    inner: Mutex<RelayInner>,
}

struct RelayInner {
    buffer: VecDeque<u8>,
    broadcast: broadcast::Sender<Vec<u8>>,
}

impl TerminalRelay {
    pub fn new(broadcast_capacity: usize) -> Arc<Self> {
        let (tx, _) = broadcast::channel(broadcast_capacity);
        Arc::new(Self {
            inner: Mutex::new(RelayInner {
                buffer: VecDeque::with_capacity(REPLAY_BUFFER_SIZE),
                broadcast: tx,
            }),
        })
    }

    /// Publish a chunk of PTY output: append to the replay buffer (evicting
    /// oldest bytes past the cap) and fan out to live subscribers.
    pub fn publish(&self, data: Vec<u8>) {
        let mut inner = self.inner.lock().unwrap();
        inner.buffer.extend(data.iter());
        while inner.buffer.len() > REPLAY_BUFFER_SIZE {
            inner.buffer.pop_front();
        }
        // Broadcast with no subscribers is the documented design of
        // TerminalOutputQueue: the replay buffer above is what new
        // subscribers consume on subscribe; live broadcast is best-effort.
        let _ = inner.broadcast.send(data); // channel-closed-ok: replay-buffer-is-source-of-truth
    }

    /// Subscribe a new client: returns the current replay snapshot plus a
    /// live receiver. Atomic vs. `publish`, so the caller sees either
    /// "snapshot only" or "snapshot + subsequent live bytes" with no
    /// duplicates and no gaps.
    pub fn subscribe(&self) -> (Vec<u8>, broadcast::Receiver<Vec<u8>>) {
        let inner = self.inner.lock().unwrap();
        let rx = inner.broadcast.subscribe();
        let snapshot: Vec<u8> = inner.buffer.iter().copied().collect();
        drop(inner);
        (snapshot, rx)
    }
}

#[cfg(test)]
mod tests;
