//! Guest-side handling of `HostToGuest::Shutdown`.
//!
//! The terminal shell is an interactive bash, which ignores SIGTERM by
//! design (so `kill 0` cannot take the shell down). The old path sent
//! SIGTERM, slept the whole grace period, then SIGKILLed: every shutdown
//! paid the full grace and bash never got to save its history. SIGHUP is the
//! signal a closing terminal sends; interactive bash saves history, hangs
//! up its jobs, and exits on it within milliseconds. We wait for that exit
//! instead of sleeping, and the grace period is only the ceiling.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use nix::sys::signal::{kill, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::Pid;

/// How often the child is checked for exit while the grace period runs, and
/// how often a `HostShutdown` wait re-checks its flag.
const CHILD_EXIT_POLL: Duration = Duration::from_millis(5);

/// How long the terminal bridge keeps the connection open after the shell
/// exits during a host shutdown, waiting for `ShutdownComplete` to be
/// written: the rest of the grace period plus the final sync and the write.
pub(crate) const HOLD_FOR_REPORT: Duration = Duration::from_secs(capsem_proto::SHUTDOWN_GRACE_SECS + 1);

/// How long the control loop waits for the writer to confirm the report
/// went out before giving up on it.
pub(crate) const REPORT_WRITE_WAIT: Duration = Duration::from_secs(1);

/// One host-initiated shutdown, shared by the three threads that must agree
/// on its order. The control loop sets `requested` before it hangs up the
/// shell; the terminal bridge, which otherwise treats the shell's exit as
/// the end of the connection, then holds the connection until the writer
/// sets `reported` after writing `GuestToHost::ShutdownComplete`. Without
/// this the bridge tore the control socket down while the report was still
/// queued, the agent reconnected to a host that was already stopping the
/// VM, and every stop waited out the host's timeout.
#[derive(Default)]
pub(crate) struct HostShutdown {
    requested: AtomicBool,
    reported: AtomicBool,
}

impl HostShutdown {
    pub(crate) fn request(&self) {
        self.requested.store(true, Ordering::SeqCst);
    }

    pub(crate) fn is_requested(&self) -> bool {
        self.requested.load(Ordering::SeqCst)
    }

    pub(crate) fn mark_reported(&self) {
        self.reported.store(true, Ordering::SeqCst);
    }

    /// Wait until `mark_reported` has been called, at most `bound`.
    pub(crate) fn wait_reported(&self, bound: Duration) -> bool {
        let deadline = Instant::now() + bound;
        loop {
            if self.reported.load(Ordering::SeqCst) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(CHILD_EXIT_POLL);
        }
    }
}

/// How the terminal shell ended during shutdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShellEnd {
    /// Exited on its own after SIGHUP, within `elapsed`.
    Exited { elapsed: Duration },
    /// Still alive at the end of the grace period and killed.
    Killed { grace: Duration },
}

impl fmt::Display for ShellEnd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exited { elapsed } => write!(f, "shell exited after {}ms", elapsed.as_millis()),
            Self::Killed { grace } => write!(f, "shell killed after {}ms grace", grace.as_millis()),
        }
    }
}

/// Flush dirty pages, hang up the terminal shell, wait for it to exit
/// (bounded by `grace`, SIGKILL past it), then flush again so whatever the
/// shell wrote on its way out (history, trap output) reaches the disk.
pub(crate) fn end_terminal_shell(child: Pid, grace: Duration) -> ShellEnd {
    sync_disks();
    let end = hang_up_and_wait(child, grace);
    sync_disks();
    end
}

/// SIGHUP the shell and wait for it to be reaped, at most `grace`; SIGKILL
/// past that. Separate from the disk syncs so its timing can be tested
/// without measuring the host's dirty-page backlog.
pub(crate) fn hang_up_and_wait(child: Pid, grace: Duration) -> ShellEnd {
    let started = Instant::now();
    let _ = kill(child, Signal::SIGHUP);
    if wait_for_exit(child, started + grace) {
        return ShellEnd::Exited {
            elapsed: started.elapsed(),
        };
    }
    let _ = kill(child, Signal::SIGKILL);
    // SIGKILL cannot be ignored; the reap is only waiting for the kernel.
    wait_for_exit(child, Instant::now() + grace);
    ShellEnd::Killed { grace }
}

/// Poll `waitpid` until the child is reaped or `deadline` passes. A child
/// that was already reaped elsewhere (`ECHILD`) counts as exited: there is
/// nothing left to wait for.
fn wait_for_exit(child: Pid, deadline: Instant) -> bool {
    loop {
        match waitpid(child, Some(WaitPidFlag::WNOHANG)) {
            Ok(WaitStatus::StillAlive) => {}
            Ok(_) | Err(nix::errno::Errno::ECHILD) => return true,
            Err(nix::errno::Errno::EINTR) => continue,
            Err(e) => {
                eprintln!("[capsem-agent] shutdown: waitpid({child}) failed: {e}");
                return false;
            }
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(CHILD_EXIT_POLL);
    }
}

fn sync_disks() {
    // SAFETY: sync(2) takes no arguments and cannot fail.
    unsafe {
        nix::libc::sync();
    }
}

#[cfg(test)]
mod tests;
