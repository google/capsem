//! Owned descriptor operations with atomic inheritance guarantees.

use std::io;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::time::Duration;

use nix::errno::Errno;
use nix::fcntl::{fcntl, FcntlArg, OFlag};
use nix::sys::socket::{self, Shutdown};

use super::errno;

/// Close every inherited descriptor except standard streams.
///
/// # Safety
/// Call only at process entry, before any other code owns descriptors above 2
/// or another thread can open and reuse them.
pub unsafe fn close_inherited_descriptors() -> io::Result<()> {
    unsafe { close_inherited_descriptors_except(&[]) }
}

/// Close ambient inherited descriptors while retaining explicit grants.
///
/// # Safety
/// Call only at process entry, before any other code owns descriptors above 2
/// or another thread can open and reuse them. Every descriptor in `preserved`
/// must be an intentional grant whose ownership the caller establishes next.
pub unsafe fn close_inherited_descriptors_except(preserved: &[RawFd]) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    let directory = "/dev/fd";
    #[cfg(not(target_os = "macos"))]
    let directory = "/proc/self/fd";
    let mut descriptors = Vec::new();
    for entry in std::fs::read_dir(directory)? {
        let name = entry?.file_name();
        let fd: RawFd = name
            .to_str()
            .ok_or_else(|| io::Error::other("invalid descriptor name"))?
            .parse()
            .map_err(io::Error::other)?;
        if should_close_inherited_descriptor(fd, preserved) {
            descriptors.push(fd);
        }
    }
    for fd in descriptors {
        // The caller guarantees no live Rust owner or fd reuse. The directory
        // iterator itself has closed; EBADF for that fd is expected.
        match nix::unistd::close(fd) {
            Ok(()) | Err(Errno::EBADF) => {}
            Err(error) => return Err(errno::io(error)),
        }
    }
    Ok(())
}

fn should_close_inherited_descriptor(fd: RawFd, preserved: &[RawFd]) -> bool {
    fd > 2 && !preserved.contains(&fd)
}

/// Which half of a connected socket to close.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SocketShutdown {
    Read,
    Write,
    Both,
}

impl SocketShutdown {
    fn as_nix(self) -> Shutdown {
        match self {
            Self::Read => Shutdown::Read,
            Self::Write => Shutdown::Write,
            Self::Both => Shutdown::Both,
        }
    }
}

/// Duplicate a borrowed descriptor into independently owned storage.
///
/// `FD_CLOEXEC` is applied by the duplication syscall itself, leaving no
/// fork-to-exec window in which another thread can leak the descriptor.
pub fn duplicate(fd: BorrowedFd<'_>) -> io::Result<OwnedFd> {
    let raw = retry_eintr(|| fcntl(fd.as_raw_fd(), FcntlArg::F_DUPFD_CLOEXEC(0))).map_err(errno::io)?;
    // SAFETY: F_DUPFD_CLOEXEC returned a new descriptor owned by this call.
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}

/// Set or clear `O_NONBLOCK`, returning its previous state.
///
/// All unrelated descriptor status flags are preserved. An interrupted
/// `fcntl` is retried; other errno values are returned unchanged.
pub fn set_nonblocking(fd: BorrowedFd<'_>, enabled: bool) -> io::Result<bool> {
    let raw_flags = retry_eintr(|| fcntl(fd.as_raw_fd(), FcntlArg::F_GETFL)).map_err(errno::io)?;
    let flags = OFlag::from_bits_truncate(raw_flags);
    let was_enabled = flags.contains(OFlag::O_NONBLOCK);
    let updated = if enabled {
        flags | OFlag::O_NONBLOCK
    } else {
        flags - OFlag::O_NONBLOCK
    };
    if updated != flags {
        retry_eintr(|| fcntl(fd.as_raw_fd(), FcntlArg::F_SETFL(updated))).map_err(errno::io)?;
    }
    Ok(was_enabled)
}

/// Wait until reading can make progress, including observing end-of-file.
pub fn wait_readable(fd: BorrowedFd<'_>, timeout: Duration) -> io::Result<bool> {
    let timeout_ms = timeout.as_nanos().saturating_add(999_999) / 1_000_000;
    let timeout_ms = i32::try_from(timeout_ms)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "poll timeout is too large"))?;
    let timeout = nix::poll::PollTimeout::try_from(timeout_ms)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "poll timeout is too large"))?;
    let mut descriptors = [nix::poll::PollFd::new(
        fd,
        nix::poll::PollFlags::POLLIN | nix::poll::PollFlags::POLLHUP,
    )];
    retry_eintr(|| nix::poll::poll(&mut descriptors, timeout))
        .map(|ready| ready > 0)
        .map_err(errno::io)
}

/// Shut down one or both halves of a connected socket.
pub fn shutdown(fd: BorrowedFd<'_>, how: SocketShutdown) -> io::Result<()> {
    socket::shutdown(fd.as_raw_fd(), how.as_nix()).map_err(errno::io)
}

/// Mark a TCP socket for reset when its last descriptor closes. Returns false
/// for other stream families. Every holder must close without first sending FIN.
pub fn tcp_reset_on_close(fd: BorrowedFd<'_>) -> io::Result<bool> {
    configure_tcp_linger(fd, true)
}

fn configure_tcp_linger(fd: BorrowedFd<'_>, enabled: bool) -> io::Result<bool> {
    use socket::SockaddrLike;
    if socket::getsockopt(&fd, socket::sockopt::SockType).map_err(errno::io)? != socket::SockType::Stream {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "reset requires a stream socket",
        ));
    }
    let address = socket::getsockname::<socket::SockaddrStorage>(fd.as_raw_fd()).map_err(errno::io)?;
    if !matches!(
        address.family(),
        Some(socket::AddressFamily::Inet | socket::AddressFamily::Inet6)
    ) {
        return Ok(false);
    }
    let linger = libc::linger {
        l_onoff: i32::from(enabled),
        l_linger: 0,
    };
    retry_eintr(|| socket::setsockopt(&fd, socket::sockopt::Linger, &linger)).map_err(errno::io)?;
    Ok(true)
}

/// Revoke a TCP connection immediately, even while another process retains a
/// duplicate descriptor. The trusted endpoint owner calls this, not the router.
pub fn reset_tcp(fd: BorrowedFd<'_>) -> io::Result<bool> {
    match tcp_reset_on_close(fd) {
        Ok(false) => return Ok(false),
        Ok(true) => {}
        Err(error) if already_torn_down(&error) => return Ok(true),
        Err(error) => return Err(error),
    }
    let result = retry_eintr(|| {
        #[cfg(target_os = "macos")]
        // SAFETY: disconnectx acts only on this borrowed socket. Linger zero
        // makes XNU tcp_disconnect use tcp_drop instead of sending FIN.
        let result = unsafe { libc::disconnectx(fd.as_raw_fd(), libc::SAE_ASSOCID_ANY, libc::SAE_CONNID_ANY) };
        #[cfg(target_os = "linux")]
        let result = {
            let address = libc::sockaddr {
                sa_family: libc::AF_UNSPEC as _,
                sa_data: [0; 14],
            };
            // SAFETY: AF_UNSPEC disconnects this existing TCP socket; the
            // initialized stack address is valid for this synchronous call.
            unsafe { libc::connect(fd.as_raw_fd(), &address, std::mem::size_of_val(&address) as _) }
        };
        if result == 0 {
            Ok(())
        } else {
            Err(Errno::last())
        }
    })
    .map_err(errno::io);
    match result {
        Ok(()) => Ok(true),
        Err(error) if already_torn_down(&error) => Ok(true),
        Err(error) => Err(error),
    }
}

/// A connection the peer already reset has nothing left to reset: XNU
/// answers EINVAL to both the linger option and `disconnectx` once the
/// socket left ESTABLISHED, Linux ENOTCONN or EPIPE. The caller wanted the
/// peer gone and it is; reporting an error here once ended a VM's whole
/// control link when a host client reset sixteen streams at once.
fn already_torn_down(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(libc::EINVAL | libc::ENOTCONN | libc::ECONNRESET | libc::EPIPE)
    )
}

/// Restore default close behavior after a TCP stream completed normally.
pub fn tcp_clear_reset_on_close(fd: BorrowedFd<'_>) -> io::Result<bool> {
    configure_tcp_linger(fd, false)
}

/// Fix socket queue sizes instead of allowing TCP receive/send autotuning.
/// Linux accounts up to twice the requested bytes for TCP bookkeeping. VSOCK
/// uses separate credit-buffer options, so those are fixed as well on Linux.
pub fn set_stream_buffers(fd: BorrowedFd<'_>, bytes: usize) -> io::Result<()> {
    if bytes == 0 || bytes > i32::MAX as usize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid socket buffer size",
        ));
    }
    socket::setsockopt(&fd, socket::sockopt::SndBuf, &bytes).map_err(errno::io)?;
    socket::setsockopt(&fd, socket::sockopt::RcvBuf, &bytes).map_err(errno::io)?;
    #[cfg(target_os = "linux")]
    {
        use socket::SockaddrLike;
        let address = socket::getsockname::<socket::SockaddrStorage>(fd.as_raw_fd()).map_err(errno::io)?;
        if address.family() == Some(socket::AddressFamily::Vsock) {
            let bytes = bytes as u64;
            // Linux uapi/linux/vm_sockets.h: MIN_SIZE=1, MAX_SIZE=2, SIZE=0.
            // Fix both bounds before SIZE so transport credit cannot grow.
            for option in [1, 2, 0] {
                retry_eintr(|| {
                    // SAFETY: setsockopt reads this initialized u64 synchronously.
                    if unsafe {
                        libc::setsockopt(
                            fd.as_raw_fd(),
                            libc::AF_VSOCK,
                            option,
                            (&bytes as *const u64).cast(),
                            std::mem::size_of::<u64>() as libc::socklen_t,
                        )
                    } == 0
                    {
                        Ok(())
                    } else {
                        Err(Errno::last())
                    }
                })
                .map_err(errno::io)?;
            }
        }
    }
    Ok(())
}

/// What the kernel holds for one connected stream socket, for saying which
/// end of a stalled copy stopped moving. Each count is read on its own and is
/// `None` where the platform or socket family cannot answer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StreamQueues {
    /// Received and not yet read by this process.
    pub unread: Option<u32>,
    /// Written and not yet acknowledged by the peer (TCP), or not yet read
    /// by it (Unix stream).
    pub unsent: Option<u32>,
    /// TCP only: written but never transmitted, because the peer's receive
    /// window is shut. Equal to `unsent` means the peer stopped reading.
    pub untransmitted: Option<u32>,
    /// TCP only: zero-window probes sent without an answer opening the
    /// window, and the probe timer's backoff exponent.
    pub probes: Option<u8>,
    pub backoff: Option<u8>,
}

/// The queue-length ioctls; the macro makes every wrapper public.
mod queue_ioctls {
    nix::ioctl_read_bad!(input, libc::FIONREAD, libc::c_int);
    #[cfg(target_os = "linux")]
    nix::ioctl_read_bad!(output, libc::TIOCOUTQ, libc::c_int);
    #[cfg(target_os = "linux")]
    nix::ioctl_read_bad!(untransmitted, libc::SIOCOUTQNSD, libc::c_int);
}

/// Read a stream socket's kernel queues. Never fails: a count the kernel
/// will not give is left out, since this only ever explains a stall.
pub fn stream_queues(fd: BorrowedFd<'_>) -> StreamQueues {
    let count = |query: unsafe fn(libc::c_int, *mut libc::c_int) -> nix::Result<libc::c_int>| {
        let mut value: libc::c_int = 0;
        // SAFETY: each query is a read-only ioctl writing one c_int into
        // `value`, which outlives this synchronous call.
        retry_eintr(|| unsafe { query(fd.as_raw_fd(), &mut value) })
            .ok()
            .and_then(|_| u32::try_from(value).ok())
    };
    #[cfg_attr(not(target_os = "linux"), allow(unused_mut))]
    let mut queues = StreamQueues {
        unread: count(queue_ioctls::input),
        ..StreamQueues::default()
    };
    #[cfg(target_os = "linux")]
    {
        queues.unsent = count(queue_ioctls::output);
        if tcp_stream(fd) {
            queues.untransmitted = count(queue_ioctls::untransmitted);
            if let Some(info) = tcp_info(fd) {
                queues.probes = Some(info.tcpi_probes);
                queues.backoff = Some(info.tcpi_backoff);
            }
        }
    }
    queues
}

#[cfg(target_os = "linux")]
fn tcp_stream(fd: BorrowedFd<'_>) -> bool {
    use socket::SockaddrLike;
    socket::getsockname::<socket::SockaddrStorage>(fd.as_raw_fd()).is_ok_and(|address| {
        matches!(
            address.family(),
            Some(socket::AddressFamily::Inet | socket::AddressFamily::Inet6)
        )
    })
}

#[cfg(target_os = "linux")]
fn tcp_info(fd: BorrowedFd<'_>) -> Option<libc::tcp_info> {
    // SAFETY: tcp_info is plain integers, for which all-zero is valid.
    let mut info: libc::tcp_info = unsafe { std::mem::zeroed() };
    let mut length = std::mem::size_of_val(&info) as libc::socklen_t;
    retry_eintr(|| {
        // SAFETY: the kernel writes at most `length` bytes into `info`, both
        // of which outlive this synchronous call.
        let result = unsafe {
            libc::getsockopt(
                fd.as_raw_fd(),
                libc::IPPROTO_TCP,
                libc::TCP_INFO,
                (&mut info as *mut libc::tcp_info).cast(),
                &mut length,
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(Errno::last())
        }
    })
    .ok()
    .map(|()| info)
}

/// Reject files, listeners and datagram sockets before adopting a relay stream.
pub fn validate_connected_stream(fd: BorrowedFd<'_>) -> io::Result<()> {
    if socket::getsockopt(&fd, socket::sockopt::SockType).map_err(errno::io)? != socket::SockType::Stream {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "router requires stream sockets",
        ));
    }
    socket::getpeername::<socket::SockaddrStorage>(fd.as_raw_fd()).map_err(errno::io)?;
    Ok(())
}

fn retry_eintr<T>(mut operation: impl FnMut() -> Result<T, Errno>) -> Result<T, Errno> {
    loop {
        match operation() {
            Err(Errno::EINTR) => {}
            result => return result,
        }
    }
}

#[cfg(test)]
mod tests;
