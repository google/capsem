//! Platform-agnostic vsock utilities: output coalescing and port constants.

// Re-export protocol types and port constants from capsem-proto.
pub use capsem_proto::{
    decode_guest_msg, decode_host_msg, encode_guest_msg, encode_host_msg, max_frame_size,
    GuestToHost, HostToGuest, MAX_FRAME_SIZE, VSOCK_PORT_CONTROL, VSOCK_PORT_EXEC,
    VSOCK_PORT_LIFECYCLE, VSOCK_PORT_SNI_PROXY, VSOCK_PORT_TERMINAL,
};

// ---------------------------------------------------------------------------
// Output coalescing buffer
// ---------------------------------------------------------------------------

/// Default coalescing time window (5ms = 200 fps).
const COALESCE_WINDOW_MS: u64 = 5;
/// Default coalescing size cap (10MB).
const COALESCE_MAX_BYTES: usize = 10_485_760;

/// Coalesces small chunks into larger batches to prevent IPC saturation.
///
/// Collects incoming data and flushes when either the time window expires
/// or the size cap is reached. The actual async loop lives in the app layer;
/// this struct holds the policy and buffer.
pub struct CoalesceBuffer {
    buf: Vec<u8>,
    max_bytes: usize,
    window_ms: u64,
}

impl CoalesceBuffer {
    /// Create a new coalescing buffer with default settings (5ms / 10MB).
    pub fn new() -> Self {
        Self {
            buf: Vec::with_capacity(COALESCE_MAX_BYTES),
            max_bytes: COALESCE_MAX_BYTES,
            window_ms: COALESCE_WINDOW_MS,
        }
    }

    /// Create with custom thresholds (for testing).
    pub fn with_limits(max_bytes: usize, window_ms: u64) -> Self {
        Self {
            buf: Vec::with_capacity(max_bytes),
            max_bytes,
            window_ms,
        }
    }

    /// Push a chunk into the buffer. Returns `true` if the size cap has been
    /// reached and the caller should flush immediately.
    pub fn push(&mut self, data: &[u8]) -> bool {
        self.buf.extend_from_slice(data);
        self.buf.len() >= self.max_bytes
    }

    /// Take the coalesced data out, leaving the buffer empty with
    /// pre-allocated capacity for the next batch.
    pub fn take(&mut self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.max_bytes);
        std::mem::swap(&mut self.buf, &mut out);
        out
    }

    /// Current buffered byte count.
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Whether the size cap has been reached.
    pub fn is_full(&self) -> bool {
        self.buf.len() >= self.max_bytes
    }

    /// The coalescing time window in milliseconds.
    pub fn window_ms(&self) -> u64 {
        self.window_ms
    }

    /// The size cap in bytes.
    pub fn max_bytes(&self) -> usize {
        self.max_bytes
    }

    /// Pass buffered data to a closure, then clear in place.
    /// Zero-allocation: the buffer's capacity is preserved across flushes.
    pub fn flush_to<F, R>(&mut self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        let r = f(&self.buf);
        self.buf.clear();
        r
    }
}

impl Default for CoalesceBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
