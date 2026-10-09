//! Process identity, liveness, and signalling.

use std::io;
use std::num::NonZeroI32;

use nix::errno::Errno;
use nix::sys::signal::{kill, killpg, Signal as NixSignal};
use nix::unistd::Pid;

use super::errno;

/// Erase the inherited environment and detach it from the C process image.
///
/// `remove_var` alone leaves the original stack bytes visible through Linux
/// `/proc/<pid>/environ`. This scrubs those bytes before emptying `environ`.
///
/// # Safety
/// Call only at process entry, before any library can replace environment
/// pointers and before another thread can read or mutate the environment.
pub unsafe fn clear_inherited_environment() {
    let environment = inherited_environment();
    if environment.is_null() {
        return;
    }
    let mut entry = environment;
    while !unsafe { *entry }.is_null() {
        let value = unsafe { *entry };
        // SAFETY: process-entry `environ` is a null-terminated array of
        // writable, NUL-terminated inherited strings. No other thread exists.
        let length = unsafe { std::ffi::CStr::from_ptr(value) }.to_bytes().len();
        unsafe { std::ptr::write_bytes(value.cast::<u8>(), 0, length) };
        entry = unsafe { entry.add(1) };
    }
    // libc and Rust observe an empty list without retaining scrubbed entries.
    unsafe { *environment = std::ptr::null_mut() };
}

#[cfg(target_os = "linux")]
fn inherited_environment() -> *mut *mut libc::c_char {
    unsafe extern "C" {
        static mut environ: *mut *mut libc::c_char;
    }
    // SAFETY: the caller upholds process-entry exclusivity.
    unsafe { environ }
}

#[cfg(target_os = "macos")]
fn inherited_environment() -> *mut *mut libc::c_char {
    unsafe extern "C" {
        fn _NSGetEnviron() -> *mut *mut *mut libc::c_char;
    }
    // SAFETY: Darwin returns the address of this process's environ pointer.
    unsafe { *_NSGetEnviron() }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn inherited_environment() -> *mut *mut libc::c_char {
    std::ptr::null_mut()
}

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

/// Whether `kill(pid, 0)` is permitted, denied, or names no process.
/// Unlike [`probe`], this preserves `EPERM` for sandbox attestation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignalProbe {
    Allowed,
    Denied,
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

/// One-minute system load, or `None` if the host cannot provide it.
///
/// The native Unix interface works on macOS as well as Linux, without procfs
/// or a subprocess that may be unavailable inside the gate's sandbox.
pub fn load_average() -> Option<f64> {
    let mut load = [0.0];
    // SAFETY: the buffer holds the one double requested and remains writable
    // for the call; getloadavg does not retain its pointer.
    let observed = unsafe { libc::getloadavg(load.as_mut_ptr(), 1) };
    (observed == 1).then_some(load[0])
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

/// Probe signal authority without folding permission denial into liveness.
pub fn probe_signal_authority(pid: ProcessId) -> io::Result<SignalProbe> {
    classify_signal_probe(kill(pid.as_nix(), None))
}

fn classify_signal_probe(result: Result<(), Errno>) -> io::Result<SignalProbe> {
    match result {
        Ok(()) => Ok(SignalProbe::Allowed),
        Err(Errno::EPERM) => Ok(SignalProbe::Denied),
        Err(Errno::ESRCH) => Ok(SignalProbe::Gone),
        Err(error) => Err(errno::io(error)),
    }
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
    let result = killpg(leader.as_nix(), signal.as_nix());
    // Darwin's killpg1 skips zombies and returns EPERM when none remain
    // signalable. Only excuse that error if the sole member is our exited,
    // unreaped child; another member or an unreadable table keeps the error.
    #[cfg(target_os = "macos")]
    if result == Err(Errno::EPERM) && lone_exited_child_group(leader) {
        return Ok(SignalOutcome::Gone);
    }
    classify_signal(result)
}

#[cfg(target_os = "macos")]
fn lone_exited_child_group(leader: ProcessId) -> bool {
    if !child_has_exited(leader).unwrap_or(false) {
        return false;
    }
    // Two slots distinguish one member from a truncated multi-member list.
    // libproc includes zombies and returns a count of PIDs, not bytes.
    let mut members: [libc::pid_t; 2] = [0; 2];
    // SAFETY: the buffer holds the specified number of bytes, and the group
    // ID belongs to the child whose unreaped status still reserves its PID.
    let count = unsafe {
        libc::proc_listpgrppids(
            leader.as_nix().as_raw(),
            members.as_mut_ptr().cast(),
            std::mem::size_of_val(&members) as libc::c_int,
        )
    };
    count == 1 && members[0] == leader.as_nix().as_raw()
}

/// Observe an owned child's exit without reaping it. Keeping the zombie until
/// group cleanup reserves its PID even if its descendants change groups.
pub fn child_has_exited(pid: ProcessId) -> io::Result<bool> {
    observe_child_exit(pid, true)
}

/// Wait for an owned child to exit without reaping it.
///
/// The caller retains custody of the child's PID and exit status until its
/// process handle is explicitly waited. This call blocks and therefore belongs
/// on a blocking worker when used from an async runtime.
pub fn wait_for_child_exit(pid: ProcessId) -> io::Result<()> {
    if observe_child_exit(pid, false)? {
        Ok(())
    } else {
        Err(io::Error::other("blocking child exit observation returned no status"))
    }
}

fn observe_child_exit(pid: ProcessId, nonblocking: bool) -> io::Result<bool> {
    loop {
        // SAFETY: zero is a valid initial siginfo_t; waitid writes this owned
        // buffer, and P_PID names only the validated positive child identifier.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let options = libc::WEXITED | libc::WNOWAIT | if nonblocking { libc::WNOHANG } else { 0 };
        let result = unsafe { libc::waitid(libc::P_PID, pid.get() as libc::id_t, &mut info, options) };
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
