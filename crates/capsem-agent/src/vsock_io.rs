// Shared vsock I/O helpers for guest-side binaries.
//
// Provides low-level vsock connect and fd read/write primitives.
// Used by both capsem-pty-agent and capsem-net-proxy.
//
// Included via #[path] in each binary, so not all functions are used in each.
#![allow(dead_code)]

use std::io;
use std::os::unix::io::{BorrowedFd, FromRawFd, RawFd};
use std::pin::Pin;
use std::sync::OnceLock;
use std::task::{Context, Poll};
use std::time::Duration;

use nix::libc;
use tokio::io::unix::AsyncFd;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// Host CID (always 2 for the hypervisor).
pub const VSOCK_HOST_CID: u32 = 2;
/// AF_VSOCK address family.
pub const AF_VSOCK: i32 = 40;

static VSOCK_PORT_OFFSET: OnceLock<u32> = OnceLock::new();

#[repr(C)]
pub struct SockaddrVm {
    pub svm_family: libc::sa_family_t,
    pub svm_reserved1: u16,
    pub svm_port: u32,
    pub svm_cid: u32,
    pub svm_flags: u8,
    pub svm_zero: [u8; 3],
}

/// I/O timeout for vsock read/write operations. If a single syscall blocks
/// longer than this, it returns EAGAIN instead of hanging forever.
/// 30s is generous -- vsock to hypervisor should drain in milliseconds.
const IO_TIMEOUT_SECS: i64 = 30;

/// Connect to a vsock port on the given CID.
///
/// Sets SO_SNDTIMEO and SO_RCVTIMEO so that blocking read/write calls
/// return EAGAIN after IO_TIMEOUT_SECS instead of hanging indefinitely
/// if the host stops draining the buffer.
pub fn vsock_connect(cid: u32, port: u32) -> io::Result<RawFd> {
    let fd = unsafe { libc::socket(AF_VSOCK, libc::SOCK_STREAM, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let physical_port = physical_vsock_port(port, guest_vsock_port_offset())?;

    let addr = SockaddrVm {
        svm_family: AF_VSOCK as libc::sa_family_t,
        svm_reserved1: 0,
        svm_port: physical_port,
        svm_cid: cid,
        svm_flags: 0,
        svm_zero: [0; 3],
    };

    let ret = unsafe {
        libc::connect(
            fd,
            &addr as *const SockaddrVm as *const libc::sockaddr,
            std::mem::size_of::<SockaddrVm>() as libc::socklen_t,
        )
    };
    if ret < 0 {
        let err = io::Error::last_os_error();
        unsafe {
            libc::close(fd);
        }
        return Err(err);
    }

    // Set I/O timeouts so blocking read/write return EAGAIN on stall
    // rather than hanging forever inside the kernel.
    set_io_timeouts(fd);

    Ok(fd)
}

fn guest_vsock_port_offset() -> u32 {
    *VSOCK_PORT_OFFSET.get_or_init(|| {
        std::fs::read_to_string("/proc/cmdline")
            .ok()
            .and_then(|cmdline| parse_vsock_port_offset(&cmdline))
            .unwrap_or(0)
    })
}

fn parse_vsock_port_offset(cmdline: &str) -> Option<u32> {
    cmdline.split_whitespace().find_map(|arg| {
        arg.strip_prefix("capsem.vsock_port_offset=")
            .and_then(|value| value.parse::<u32>().ok())
    })
}

fn physical_vsock_port(logical_port: u32, offset: u32) -> io::Result<u32> {
    logical_port.checked_add(offset).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("vsock port overflow: logical={logical_port} offset={offset}"),
        )
    })
}

/// Apply send and receive timeouts to a socket fd.
fn set_io_timeouts(fd: RawFd) {
    let timeout = Duration::from_secs(IO_TIMEOUT_SECS as u64);
    set_socket_timeout(fd, libc::SO_SNDTIMEO, timeout);
    set_socket_timeout(fd, libc::SO_RCVTIMEO, timeout);
}

/// Set one of `SO_SNDTIMEO` / `SO_RCVTIMEO`; `Duration::ZERO` disables it.
pub fn set_socket_timeout(fd: RawFd, which: libc::c_int, timeout: Duration) {
    // The field types are `time_t` / `suseconds_t`, whose aliases are
    // deprecated on musl (they widen in a future libc); infer them instead.
    let tv = libc::timeval {
        tv_sec: i64::try_from(timeout.as_secs()).unwrap_or(i64::MAX) as _,
        tv_usec: i32::try_from(timeout.subsec_micros()).unwrap_or(0).into(),
    };
    unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            which,
            &tv as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::timeval>() as libc::socklen_t,
        );
    }
}

/// Let reads on `fd` wait for as long as the peer stays connected.
///
/// The receive timeout `vsock_connect` installs is right for channels with a
/// heartbeat: silence means the host is gone. It is wrong for a transport
/// with no keepalive, where silence is the normal state -- a model thinking
/// between tool calls, or a tool call that takes longer than the timeout --
/// and where `read_exact_fd` turns the timeout into a disconnect.
pub fn clear_recv_timeout(fd: RawFd) {
    set_socket_timeout(fd, libc::SO_RCVTIMEO, Duration::ZERO);
}

/// Intern a retry label so `RetryOpts` can borrow it for `'static` without
/// leaking a fresh allocation on every reconnect.
fn static_label(label: String) -> &'static str {
    static LABELS: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());
    let mut labels = LABELS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(existing) = labels.iter().find(|existing| **existing == label) {
        return existing;
    }
    let leaked: &'static str = Box::leak(label.into_boxed_str());
    labels.push(leaked);
    leaked
}

/// Connect to a vsock port with exponential backoff retry and total timeout.
///
/// Uses the shared `RetryOpts` backoff (50ms initial, 500ms max, 30s timeout).
/// Exits the process on timeout -- vsock is required for guest operation.
pub fn vsock_connect_retry(cid: u32, port: u32, label: &str) -> RawFd {
    use capsem_proto::poll::{retry_with_backoff, RetryOpts};

    let static_label = static_label(format!("vsock-{label}"));

    match retry_with_backoff(&RetryOpts::new(static_label, Duration::from_secs(30)), || {
        vsock_connect(cid, port).ok()
    }) {
        Ok(fd) => {
            let physical_port = physical_vsock_port(port, guest_vsock_port_offset()).unwrap_or(port);
            if physical_port == port {
                eprintln!("[capsem-agent] {label} connected (port {port})");
            } else {
                eprintln!("[capsem-agent] {label} connected (logical port {port}, physical port {physical_port})");
            }
            fd
        }
        Err(e) => {
            eprintln!("[capsem-agent] {label} connect timed out: {e}");
            std::process::exit(1);
        }
    }
}

/// Write all bytes to an fd, retrying on partial writes.
///
/// Defense in depth against hangs:
/// - `Ok(0)`: treated as WriteZero (no progress) to prevent infinite loop
/// - `EAGAIN`: treated as fatal timeout (SO_SNDTIMEO fired), not retryable,
///   to prevent turning a kernel hang into a userspace spin-loop
/// - `EINTR`: retried (signal interrupted the syscall, normal)
/// - All other errors: propagated immediately
pub fn write_all_fd(fd: RawFd, data: &[u8]) -> io::Result<()> {
    let mut written = 0;
    while written < data.len() {
        match nix::unistd::write(
            unsafe { std::os::unix::io::BorrowedFd::borrow_raw(fd) },
            &data[written..],
        ) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "write returned 0 bytes (no progress)",
                ));
            }
            Ok(n) => written += n,
            Err(nix::errno::Errno::EINTR) => continue,
            Err(nix::errno::Errno::EAGAIN) => {
                // SO_SNDTIMEO fired -- host is not draining the buffer.
                // Treat as fatal, not retryable, to prevent userspace spin.
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "write timed out (host not reading)",
                ));
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// Read exactly `buf.len()` bytes from an fd, retrying on partial reads.
///
/// Defense in depth against hangs:
/// - `Ok(0)`: EOF before buffer filled, returns UnexpectedEof
/// - `EAGAIN`: treated as fatal timeout (SO_RCVTIMEO fired), not retryable
/// - `EINTR`: retried (signal interrupted the syscall, normal)
/// - All other errors: propagated immediately
pub fn read_exact_fd(fd: RawFd, buf: &mut [u8]) -> io::Result<()> {
    let mut pos = 0;
    while pos < buf.len() {
        match nix::unistd::read(fd, &mut buf[pos..]) {
            Ok(0) => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "unexpected EOF")),
            Ok(n) => pos += n,
            Err(nix::errno::Errno::EINTR) => continue,
            Err(nix::errno::Errno::EAGAIN) => {
                // SO_RCVTIMEO fired -- host is not sending data.
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "read timed out (host not writing)",
                ));
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "vsock_io/tests.rs"]
mod tests;

/// Async wrapper for a connected vsock `RawFd`, usable wherever tokio wants
/// an `AsyncRead + AsyncWrite`. Shared by the net proxy and the DNS proxy.
pub struct AsyncVsock {
    inner: AsyncFd<std::os::unix::net::UnixStream>,
    fd: RawFd,
}

impl AsyncVsock {
    /// Take ownership of `fd`. On every path, including an error, the fd
    /// belongs to this function afterwards: `from_raw_fd` owns it and
    /// `AsyncFd::new` drops (closes) the stream when registration fails.
    /// The caller must not close it again; a second close on a multi-threaded
    /// runtime can hit a number another connection has just been handed.
    pub fn new(fd: RawFd) -> io::Result<Self> {
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFL, 0);
            libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
        // We wrap it in a UnixStream to be able to use AsyncFd,
        // although it's actually an AF_VSOCK socket.
        let std_stream = unsafe { std::os::unix::net::UnixStream::from_raw_fd(fd) };
        Ok(Self {
            inner: AsyncFd::new(std_stream)?,
            fd,
        })
    }
}

impl AsyncRead for AsyncVsock {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        loop {
            let mut guard = match self.inner.poll_read_ready(cx) {
                Poll::Ready(Ok(guard)) => guard,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            };

            let unfilled = buf.initialize_unfilled();
            match nix::unistd::read(self.fd, unfilled) {
                Ok(n) => {
                    buf.advance(n);
                    return Poll::Ready(Ok(()));
                }
                Err(nix::errno::Errno::EAGAIN) => {
                    guard.clear_ready();
                }
                Err(e) => return Poll::Ready(Err(e.into())),
            }
        }
    }
}

impl AsyncWrite for AsyncVsock {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        loop {
            let mut guard = match self.inner.poll_write_ready(cx) {
                Poll::Ready(Ok(guard)) => guard,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            };

            match nix::unistd::write(unsafe { BorrowedFd::borrow_raw(self.fd) }, buf) {
                Ok(n) => return Poll::Ready(Ok(n)),
                Err(nix::errno::Errno::EAGAIN) => {
                    guard.clear_ready();
                }
                Err(e) => return Poll::Ready(Err(e.into())),
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let ret = unsafe { libc::shutdown(self.fd, libc::SHUT_WR) };
        if ret == 0 {
            Poll::Ready(Ok(()))
        } else {
            Poll::Ready(Err(io::Error::last_os_error()))
        }
    }
}

// No custom Drop: inner AsyncFd<UnixStream> owns the fd via from_raw_fd
// and closes it automatically. Manual libc::close would double-close.
