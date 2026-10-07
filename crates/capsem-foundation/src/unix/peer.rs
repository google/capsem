//! Kernel identity of a connected Unix peer, independent of protocol claims.
use std::io;
use std::os::fd::BorrowedFd;

use nix::sys::socket::{self, SockaddrLike};

use super::process::ProcessId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PeerIdentity {
    pub pid: ProcessId,
    pub uid: u32,
}

/// Identity recorded by the kernel when this Unix stream was connected.
/// Descriptor passing delegates that connection; it does not change this
/// identity. This proves neither executable bytes nor a live grant generation.
pub fn identity(fd: BorrowedFd<'_>) -> io::Result<PeerIdentity> {
    super::fd::validate_connected_stream(fd)?;
    use std::os::fd::AsRawFd;
    let address = socket::getpeername::<socket::SockaddrStorage>(fd.as_raw_fd()).map_err(super::errno::io)?;
    if address.family() != Some(socket::AddressFamily::Unix) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "peer identity requires a connected Unix stream",
        ));
    }
    platform_identity(fd)
}

#[cfg(target_os = "linux")]
fn platform_identity(fd: BorrowedFd<'_>) -> io::Result<PeerIdentity> {
    let credentials = socket::getsockopt(&fd, socket::sockopt::PeerCredentials).map_err(super::errno::io)?;
    checked_identity(credentials.pid(), credentials.uid())
}

#[cfg(target_os = "macos")]
fn platform_identity(fd: BorrowedFd<'_>) -> io::Result<PeerIdentity> {
    let pid = socket::getsockopt(&fd, socket::sockopt::LocalPeerPid).map_err(super::errno::io)?;
    let credentials = socket::getsockopt(&fd, socket::sockopt::LocalPeerCred).map_err(super::errno::io)?;
    checked_identity(pid, credentials.uid())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn platform_identity(_fd: BorrowedFd<'_>) -> io::Result<PeerIdentity> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Unix peer identity unavailable",
    ))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn checked_identity(pid: i32, uid: u32) -> io::Result<PeerIdentity> {
    let pid = u32::try_from(pid)
        .ok()
        .and_then(|pid| ProcessId::try_from(pid).ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "kernel returned an invalid Unix peer PID"))?;
    Ok(PeerIdentity { pid, uid })
}

/// Require the exact process and user granted by the trusted supervisor.
pub fn require(fd: BorrowedFd<'_>, expected: PeerIdentity) -> io::Result<()> {
    if identity(fd)? != expected {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Unix peer is not the granted process",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
