//! Process identity, liveness, and signalling.

use std::io;
use std::num::NonZeroI32;

use nix::errno::Errno;
use nix::sys::signal::{kill, killpg, Signal as NixSignal};
use nix::unistd::Pid;

use super::errno;

/// A positive process identifier representable by the host kernel ABI.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ProcessId(NonZeroI32);

impl ProcessId {
    /// Return the identifier as the unsigned value used by Capsem protocols.
    pub fn get(self) -> u32 {
        self.0.get() as u32
    }

    fn as_nix(self) -> Pid {
        Pid::from_raw(self.0.get())
    }
}

impl TryFrom<u32> for ProcessId {
    type Error = io::Error;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        let raw = i32::try_from(value)
            .ok()
            .and_then(NonZeroI32::new)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("invalid process id {value}")))?;
        Ok(Self(raw))
    }
}

/// Result of a side-effect-free process probe.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessState {
    Alive,
    Gone,
}

/// Signals used by host-side lifecycle orchestration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Signal {
    Terminate,
    Kill,
}

impl Signal {
    fn as_nix(self) -> NixSignal {
        match self {
            Self::Terminate => NixSignal::SIGTERM,
            Self::Kill => NixSignal::SIGKILL,
        }
    }
}

/// Result of signalling a process that may have exited concurrently.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignalOutcome {
    Delivered,
    Gone,
}

/// The calling user's numeric identifier.
pub fn current_uid() -> u32 {
    nix::unistd::getuid().as_raw()
}

/// The current process's parent, or `None` for an unrepresentable kernel value.
pub fn parent_process_id() -> Option<ProcessId> {
    u32::try_from(nix::unistd::getppid().as_raw())
        .ok()
        .and_then(|raw| ProcessId::try_from(raw).ok())
}

/// Probe whether a process exists without delivering a signal.
///
/// `EPERM` proves that the process exists but belongs to another user. Only
/// `ESRCH` means it is gone; every other errno is preserved for the caller.
pub fn probe(pid: ProcessId) -> io::Result<ProcessState> {
    classify_probe(kill(pid.as_nix(), None))
}

fn classify_probe(result: Result<(), Errno>) -> io::Result<ProcessState> {
    match result {
        Ok(()) | Err(Errno::EPERM) => Ok(ProcessState::Alive),
        Err(Errno::ESRCH) => Ok(ProcessState::Gone),
        Err(error) => Err(errno::io(error)),
    }
}

/// Deliver a lifecycle signal without hiding a concurrent process exit.
pub fn send_signal(pid: ProcessId, signal: Signal) -> io::Result<SignalOutcome> {
    classify_signal(kill(pid.as_nix(), signal.as_nix()))
}

/// Signal a process group created and owned by the caller, named by its leader.
pub fn send_process_group_signal(leader: ProcessId, signal: Signal) -> io::Result<SignalOutcome> {
    classify_signal(killpg(leader.as_nix(), signal.as_nix()))
}

/// Observe an owned child's exit without reaping it. Keeping the zombie until
/// group cleanup reserves its PID even if its descendants change groups.
pub fn child_has_exited(pid: ProcessId) -> io::Result<bool> {
    loop {
        // SAFETY: zero is a valid initial siginfo_t; waitid writes this owned
        // buffer, and P_PID names only the validated positive child identifier.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                pid.get() as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result == 0 {
            // SAFETY: waitid initialized the child-status fields; an unchanged
            // zero PID means no status was available with WNOHANG.
            return Ok(unsafe { info.si_pid() } != 0);
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

fn classify_signal(result: Result<(), Errno>) -> io::Result<SignalOutcome> {
    match result {
        Ok(()) => Ok(SignalOutcome::Delivered),
        Err(Errno::ESRCH) => Ok(SignalOutcome::Gone),
        Err(error) => Err(errno::io(error)),
    }
}

#[cfg(test)]
mod tests;
