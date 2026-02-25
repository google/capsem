use std::os::unix::io::RawFd;

use anyhow::Result;
use objc2::rc::Retained;
use objc2::runtime::{Bool, ProtocolObject};
use objc2::{define_class, msg_send, AnyThread, DefinedClass, Message};
use objc2_foundation::{NSArray, NSObject, NSObjectProtocol};
use objc2_virtualization::{
    VZSocketDevice, VZVirtioSocketConnection, VZVirtioSocketDevice, VZVirtioSocketListener,
    VZVirtioSocketListenerDelegate,
};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

// Re-export protocol types from capsem-proto.
pub use capsem_proto::{
    GuestToHost, HostToGuest, MAX_FRAME_SIZE, decode_guest_msg, decode_host_msg, encode_guest_msg,
    encode_host_msg, max_frame_size,
};

/// vsock port for structured control messages (resize, heartbeat).
pub const VSOCK_PORT_CONTROL: u32 = 5000;
/// vsock port for raw PTY byte streaming (stdin/stdout).
pub const VSOCK_PORT_TERMINAL: u32 = 5001;
/// vsock port for SNI proxy (HTTPS/HTTP traffic from guest).
pub const VSOCK_PORT_SNI_PROXY: u32 = 5002;

// ---------------------------------------------------------------------------
// Output coalescing buffer
// ---------------------------------------------------------------------------

/// Default coalescing time window (10ms = 100 fps).
const COALESCE_WINDOW_MS: u64 = 10;
/// Default coalescing size cap.
const COALESCE_MAX_BYTES: usize = 65536;

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
    /// Create a new coalescing buffer with default settings (8ms / 64KB).
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

// ---------------------------------------------------------------------------
// VsockConnection: represents an accepted guest connection
// ---------------------------------------------------------------------------

/// An accepted vsock connection with its file descriptor and port info.
pub struct VsockConnection {
    pub fd: RawFd,
    pub port: u32,
    // Keep the ObjC connection alive so the fd stays valid.
    _connection: Retained<VZVirtioSocketConnection>,
}

// Safety: The fd is a valid unix file descriptor that can be used across threads.
unsafe impl Send for VsockConnection {}

// ---------------------------------------------------------------------------
// Listener delegate (ObjC bridge)
// ---------------------------------------------------------------------------

struct DelegateIvars {
    tx: mpsc::UnboundedSender<VsockConnection>,
}

define_class!(
    // Safety: NSObject has no subclassing requirements.
    #[unsafe(super(NSObject))]
    #[name = "CapsemVsockListenerDelegate"]
    #[ivars = DelegateIvars]
    struct VsockListenerDelegate;

    unsafe impl NSObjectProtocol for VsockListenerDelegate {}

    unsafe impl VZVirtioSocketListenerDelegate for VsockListenerDelegate {
        #[unsafe(method(listener:shouldAcceptNewConnection:fromSocketDevice:))]
        fn listener_should_accept(
            &self,
            _listener: &VZVirtioSocketListener,
            connection: &VZVirtioSocketConnection,
            _socket_device: &VZVirtioSocketDevice,
        ) -> Bool {
            let fd = unsafe { connection.fileDescriptor() };
            let port = unsafe { connection.destinationPort() };
            debug!(fd, port, "vsock: incoming connection");

            if fd < 0 {
                warn!("vsock: connection has invalid fd (-1), rejecting");
                return Bool::NO;
            }

            // Retain the connection object so the fd stays open.
            let retained_conn: Retained<VZVirtioSocketConnection> = connection.retain();
            let conn = VsockConnection {
                fd,
                port,
                _connection: retained_conn,
            };

            if let Err(e) = self.ivars().tx.send(conn) {
                warn!("vsock: failed to send connection to manager: {e}");
                return Bool::NO;
            }

            Bool::YES
        }
    }
);

impl VsockListenerDelegate {
    fn new(tx: mpsc::UnboundedSender<VsockConnection>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(DelegateIvars { tx });
        unsafe { msg_send![super(this), init] }
    }
}

// ---------------------------------------------------------------------------
// VsockManager
// ---------------------------------------------------------------------------

/// Manages vsock listeners on the host side.
///
/// After VM boot, call `new` with the VM's socket devices to attach listeners.
/// Accepted connections are delivered via the `accept` method.
pub struct VsockManager {
    rx: mpsc::UnboundedReceiver<VsockConnection>,
    // Keep delegates alive so they don't get deallocated.
    _delegate: Retained<VsockListenerDelegate>,
    _listeners: Vec<Retained<VZVirtioSocketListener>>,
}

// Safety: We manage thread safety through the channel.
unsafe impl Send for VsockManager {}

impl VsockManager {
    /// Create a VsockManager and register listeners on the given socket device.
    ///
    /// The socket device is obtained from `VZVirtualMachine::socketDevices()` after
    /// the VM is created. Must be called from the main thread (ObjC constraint).
    pub fn new(socket_devices: &NSArray<VZSocketDevice>, ports: &[u32]) -> Result<Self> {
        let device_count = socket_devices.count();
        if device_count == 0 {
            anyhow::bail!("no socket devices configured on VM");
        }

        // There's only one VZVirtioSocketDeviceConfiguration allowed per VM.
        let socket_device = socket_devices.objectAtIndex(0);

        // Downcast VZSocketDevice -> VZVirtioSocketDevice.
        // Safety: We only configure VZVirtioSocketDeviceConfiguration, so the
        // runtime type is always VZVirtioSocketDevice.
        let device_ref: &VZSocketDevice = &socket_device;
        let virtio_device: &VZVirtioSocketDevice =
            unsafe { &*(device_ref as *const VZSocketDevice as *const VZVirtioSocketDevice) };

        let (tx, rx) = mpsc::unbounded_channel();

        let delegate = VsockListenerDelegate::new(tx);
        let delegate_proto =
            ProtocolObject::from_retained(delegate.clone() as Retained<VsockListenerDelegate>);

        let mut listeners = Vec::new();
        for &port in ports {
            let listener = unsafe { VZVirtioSocketListener::new() };
            unsafe {
                listener.setDelegate(Some(&delegate_proto));
                virtio_device.setSocketListener_forPort(&listener, port);
            }
            info!(port, "vsock: listener registered");
            listeners.push(listener);
        }

        Ok(Self {
            rx,
            _delegate: delegate,
            _listeners: listeners,
        })
    }

    /// Receive the next accepted connection (async).
    pub async fn accept(&mut self) -> Option<VsockConnection> {
        self.rx.recv().await
    }

    /// Receive the next accepted connection (blocking).
    /// For use in non-async contexts like CLI mode.
    pub fn accept_blocking(&mut self) -> Option<VsockConnection> {
        self.rx.blocking_recv()
    }

    /// Try to receive the next accepted connection without blocking.
    /// Returns `Ok(conn)` if a connection is available, `Err` if the channel
    /// is empty or closed. For use in poll loops that must also pump CFRunLoop.
    pub fn try_accept(&mut self) -> Result<VsockConnection, tokio::sync::mpsc::error::TryRecvError> {
        self.rx.try_recv()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Constants
    // -----------------------------------------------------------------------

    #[test]
    fn port_constants_are_distinct() {
        assert_ne!(VSOCK_PORT_CONTROL, VSOCK_PORT_TERMINAL);
    }

    #[test]
    fn port_constants_are_in_expected_range() {
        assert!(VSOCK_PORT_CONTROL < 65536);
        assert!(VSOCK_PORT_TERMINAL < 65536);
    }

    #[test]
    fn max_frame_size_is_8kb() {
        assert_eq!(max_frame_size(), 8192);
    }

    // -----------------------------------------------------------------------
    // CoalesceBuffer -- basic API
    // -----------------------------------------------------------------------

    #[test]
    fn coalesce_new_is_empty() {
        let buf = CoalesceBuffer::new();
        assert!(buf.is_empty());
        assert_eq!(buf.len(), 0);
        assert!(!buf.is_full());
    }

    #[test]
    fn coalesce_default_limits() {
        let buf = CoalesceBuffer::new();
        assert_eq!(buf.max_bytes(), 65536);
        assert_eq!(buf.window_ms(), 10);
    }

    #[test]
    fn coalesce_custom_limits() {
        let buf = CoalesceBuffer::with_limits(1024, 50);
        assert_eq!(buf.max_bytes(), 1024);
        assert_eq!(buf.window_ms(), 50);
    }

    #[test]
    fn coalesce_default_trait() {
        let buf = CoalesceBuffer::default();
        assert_eq!(buf.max_bytes(), 65536);
    }

    // -----------------------------------------------------------------------
    // CoalesceBuffer -- push and take
    // -----------------------------------------------------------------------

    #[test]
    fn coalesce_push_single_chunk() {
        let mut buf = CoalesceBuffer::new();
        let full = buf.push(b"hello");
        assert!(!full);
        assert_eq!(buf.len(), 5);
        assert!(!buf.is_empty());
    }

    #[test]
    fn coalesce_push_accumulates() {
        let mut buf = CoalesceBuffer::new();
        buf.push(b"aaa");
        buf.push(b"bbb");
        buf.push(b"ccc");
        assert_eq!(buf.len(), 9);
    }

    #[test]
    fn coalesce_take_returns_accumulated_data() {
        let mut buf = CoalesceBuffer::new();
        buf.push(b"hello ");
        buf.push(b"world");
        let data = buf.take();
        assert_eq!(&data, b"hello world");
    }

    #[test]
    fn coalesce_take_resets_buffer() {
        let mut buf = CoalesceBuffer::new();
        buf.push(b"data");
        let _ = buf.take();
        assert!(buf.is_empty());
        assert_eq!(buf.len(), 0);
    }

    #[test]
    fn coalesce_take_on_empty_returns_empty_vec() {
        let mut buf = CoalesceBuffer::new();
        let data = buf.take();
        assert!(data.is_empty());
    }

    #[test]
    fn coalesce_reusable_after_take() {
        let mut buf = CoalesceBuffer::new();
        buf.push(b"batch1");
        let b1 = buf.take();
        assert_eq!(&b1, b"batch1");

        buf.push(b"batch2");
        let b2 = buf.take();
        assert_eq!(&b2, b"batch2");
    }

    // -----------------------------------------------------------------------
    // CoalesceBuffer -- size cap / backpressure
    // -----------------------------------------------------------------------

    #[test]
    fn coalesce_signals_full_at_cap() {
        let mut buf = CoalesceBuffer::with_limits(10, 8);
        let full = buf.push(b"0123456789"); // exactly 10 bytes
        assert!(full);
        assert!(buf.is_full());
    }

    #[test]
    fn coalesce_signals_full_over_cap() {
        let mut buf = CoalesceBuffer::with_limits(10, 8);
        let full = buf.push(b"0123456789ABCDEF"); // 16 bytes > 10
        assert!(full);
        assert!(buf.is_full());
        // All data is still captured, even over cap.
        assert_eq!(buf.len(), 16);
    }

    #[test]
    fn coalesce_not_full_below_cap() {
        let mut buf = CoalesceBuffer::with_limits(10, 8);
        let full = buf.push(b"012345678"); // 9 bytes < 10
        assert!(!full);
        assert!(!buf.is_full());
    }

    #[test]
    fn coalesce_incremental_fill_to_cap() {
        let mut buf = CoalesceBuffer::with_limits(10, 8);
        assert!(!buf.push(b"aaa")); // 3
        assert!(!buf.push(b"bbb")); // 6
        assert!(!buf.push(b"ccc")); // 9
        assert!(buf.push(b"d"));    // 10 -- cap hit
        assert!(buf.is_full());
        let data = buf.take();
        assert_eq!(&data, b"aaabbbcccd");
    }

    #[test]
    fn coalesce_cap_resets_after_take() {
        let mut buf = CoalesceBuffer::with_limits(10, 8);
        buf.push(b"0123456789");
        assert!(buf.is_full());
        let _ = buf.take();
        assert!(!buf.is_full());
        assert!(buf.is_empty());
    }

    // -----------------------------------------------------------------------
    // CoalesceBuffer -- simulated high-throughput scenario
    // -----------------------------------------------------------------------

    #[test]
    fn coalesce_many_small_chunks() {
        // Simulate `find /` producing thousands of small lines.
        let mut buf = CoalesceBuffer::with_limits(1024, 8);
        let line = b"/usr/lib/some/path\n";
        let mut total = 0;
        let mut flush_count = 0;
        for _ in 0..200 {
            if buf.push(line) {
                let batch = buf.take();
                assert!(batch.len() >= 1024);
                total += batch.len();
                flush_count += 1;
            }
        }
        // Drain remainder.
        if !buf.is_empty() {
            total += buf.take().len();
            flush_count += 1;
        }
        assert_eq!(total, 200 * line.len());
        // With 1024 cap and 19-byte lines, expect ~4 flushes (19*54=1026 per batch).
        assert!(flush_count >= 3, "expected at least 3 flushes, got {flush_count}");
        assert!(flush_count <= 10, "expected at most 10 flushes, got {flush_count}");
    }

    #[test]
    fn coalesce_single_large_chunk_triggers_immediate_flush() {
        // A single chunk larger than the cap should signal full immediately.
        let mut buf = CoalesceBuffer::with_limits(100, 8);
        let big = vec![0x41u8; 500];
        let full = buf.push(&big);
        assert!(full);
        let data = buf.take();
        assert_eq!(data.len(), 500);
    }

    #[test]
    fn coalesce_preserves_byte_ordering() {
        let mut buf = CoalesceBuffer::with_limits(1024, 8);
        for i in 0u8..=255 {
            buf.push(&[i]);
        }
        let data = buf.take();
        assert_eq!(data.len(), 256);
        for (i, &byte) in data.iter().enumerate() {
            assert_eq!(byte, i as u8, "byte mismatch at index {i}");
        }
    }

    #[test]
    fn coalesce_zero_cap_always_full() {
        let mut buf = CoalesceBuffer::with_limits(0, 8);
        assert!(buf.is_full()); // empty but cap is 0
        let full = buf.push(b"x");
        assert!(full);
    }

    #[test]
    fn coalesce_take_preserves_capacity() {
        let mut buf = CoalesceBuffer::with_limits(1024, 8);
        buf.push(b"data");
        let _ = buf.take();
        // After take, internal buffer should still have capacity pre-allocated
        // so the next push doesn't trigger a reallocation.
        assert!(buf.buf.capacity() >= 1024);
    }

    #[test]
    fn coalesce_no_realloc_across_flushes() {
        let mut buf = CoalesceBuffer::with_limits(256, 8);
        for _ in 0..100 {
            buf.push(b"0123456789abcdef"); // 16 bytes
            if buf.is_full() {
                let _ = buf.take();
                // Capacity must be restored after take.
                assert!(
                    buf.buf.capacity() >= 256,
                    "capacity dropped to {} after take",
                    buf.buf.capacity()
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // CoalesceBuffer -- flush_to (zero-allocation)
    // -----------------------------------------------------------------------

    #[test]
    fn coalesce_flush_to_returns_data() {
        let mut buf = CoalesceBuffer::new();
        buf.push(b"hello ");
        buf.push(b"world");
        let data = buf.flush_to(|b| b.to_vec());
        assert_eq!(&data, b"hello world");
    }

    #[test]
    fn coalesce_flush_to_clears_buffer() {
        let mut buf = CoalesceBuffer::new();
        buf.push(b"data");
        buf.flush_to(|_| {});
        assert!(buf.is_empty());
        assert_eq!(buf.len(), 0);
    }

    #[test]
    fn coalesce_flush_to_preserves_capacity() {
        let mut buf = CoalesceBuffer::with_limits(1024, 8);
        buf.push(b"data");
        buf.flush_to(|_| {});
        assert!(
            buf.buf.capacity() >= 1024,
            "flush_to dropped capacity to {}",
            buf.buf.capacity()
        );
    }

    #[test]
    fn coalesce_flush_to_no_realloc_across_flushes() {
        let mut buf = CoalesceBuffer::with_limits(256, 8);
        for _ in 0..100 {
            buf.push(b"0123456789abcdef");
            if buf.is_full() {
                buf.flush_to(|_| {});
                assert!(
                    buf.buf.capacity() >= 256,
                    "capacity dropped to {} after flush_to",
                    buf.buf.capacity()
                );
            }
        }
    }
}
