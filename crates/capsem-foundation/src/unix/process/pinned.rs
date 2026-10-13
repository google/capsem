//! Stable Linux process identity. Pin first, then verify authorization against
//! fresh native identity; an earlier PID-table observation is not authority.
use super::{ProcessId, Signal, SignalOutcome};
use crate::unix::errno;
use nix::{
    errno::Errno,
    poll::{poll, PollFd, PollFlags},
};
use std::{
    io,
    os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd},
    time::Instant,
};

/// Kernel handle for one original process, retained after exit and PID reuse.
pub struct ProcessHandle {
    fd: OwnedFd,
    pid: ProcessId,
}

impl ProcessHandle {
    pub fn open(pid: ProcessId) -> io::Result<Self> {
        loop {
            // nix 0.29 has no pidfd_open wrapper. Linux atomically sets CLOEXEC.
            // SAFETY: only a validated positive PID and zero flags are passed;
            // success returns a fresh descriptor owned exclusively by this call.
            let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, pid.get() as libc::pid_t, 0u32) };
            if raw >= 0 {
                return Ok(Self {
                    fd: unsafe { OwnedFd::from_raw_fd(raw as RawFd) },
                    pid,
                });
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }

    pub fn pid(&self) -> ProcessId {
        self.pid
    }

    /// Signal the pinned process; a stale handle never targets its reused PID.
    pub fn signal(&self, signal: Signal) -> io::Result<SignalOutcome> {
        loop {
            // nix 0.29 has no pidfd_send_signal wrapper.
            // SAFETY: the descriptor is owned, the signal is fixed, and null
            // siginfo asks the kernel to construct the sender information.
            let result = unsafe {
                libc::syscall(
                    libc::SYS_pidfd_send_signal,
                    self.as_raw_fd(),
                    signal.as_nix() as libc::c_int,
                    std::ptr::null::<libc::siginfo_t>(),
                    0u32,
                )
            };
            if result == 0 {
                return Ok(SignalOutcome::Delivered);
            }
            let error = io::Error::last_os_error();
            match error.raw_os_error() {
                Some(code) if code == Errno::ESRCH as i32 => return Ok(SignalOutcome::Gone),
                Some(code) if code == Errno::EINTR as i32 => continue,
                _ => return Err(error),
            }
        }
    }

    /// Nonblocking positive native exit observation, including after reaping.
    pub fn has_exited(&self) -> io::Result<bool> {
        loop {
            match self.poll_exit(0) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                result => return result,
            }
        }
    }

    /// Blocking, deadline-bound wait. This never reaps or signals the process.
    pub fn wait_until_exit(&self, deadline: Instant) -> io::Result<bool> {
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let millis = if remaining.is_zero() {
                0
            } else {
                remaining.as_millis().clamp(1, u128::from(u16::MAX)) as u16
            };
            match self.poll_exit(millis) {
                Ok(true) => return Ok(true),
                Ok(false) => {}
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
        }
    }

    fn poll_exit(&self, millis: u16) -> io::Result<bool> {
        let mut descriptors = [PollFd::new(self.as_fd(), PollFlags::POLLIN)];
        poll(&mut descriptors, millis).map_err(errno::io)?;
        let events = descriptors[0]
            .revents()
            .ok_or_else(|| io::Error::other("invalid pidfd events"))?;
        if events.intersects(PollFlags::POLLERR | PollFlags::POLLNVAL) {
            return Err(io::Error::other("pidfd exit observation failed"));
        }
        Ok(events.intersects(PollFlags::POLLIN | PollFlags::POLLHUP))
    }
}

impl AsRawFd for ProcessHandle {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

impl AsFd for ProcessHandle {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

#[cfg(test)]
mod tests;
