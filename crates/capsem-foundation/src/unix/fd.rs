//! Owned descriptor operations with atomic inheritance guarantees.

use std::io;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd};

use nix::errno::Errno;
use nix::fcntl::{fcntl, FcntlArg, OFlag};
use nix::sys::socket::{self, Shutdown};

use super::errno;

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

/// Shut down one or both halves of a connected socket.
pub fn shutdown(fd: BorrowedFd<'_>, how: SocketShutdown) -> io::Result<()> {
    socket::shutdown(fd.as_raw_fd(), how.as_nix()).map_err(errno::io)
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
