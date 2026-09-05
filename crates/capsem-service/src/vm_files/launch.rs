//! The window `create` waits after spawning capsem-process.
//!
//! `create` must catch a synchronous launch failure (the process crashed
//! before the VM ran) without waiting for a full boot: exec and file routes
//! own the real readiness wait. It used to poll for `.ready`, written only
//! after the guest handshake 400-700 ms into boot, so the 500 ms ceiling
//! was the usual outcome and every create paid it in full.
//!
//! capsem-process now writes `.launched` as soon as the hypervisor has
//! started the VM and it is listening for IPC, about 100 ms in on Linux.
//! The wait returns on that, on `.ready`, or on the instance vanishing
//! (a crash), with the ceiling kept as the bound.
//!
//! Why the launchd-transient detection still works: on macOS the
//! `validateWithError` crash happens inside `boot_vm`, before the IPC
//! listener binds, so no sentinel exists and the reaper's removal of the
//! instance is still the first observable event -- `Crashed`, classified
//! as before.

use std::path::Path;
use std::time::Duration;

use capsem_foundation::poll::{poll_until, PollOpts};

/// The most a healthy create waits. Cold boots take longer than this to
/// become exec-ready; the bound only exists for a process that neither
/// launches nor dies.
pub(super) const LAUNCH_CEILING: Duration = Duration::from_millis(500);

#[derive(Debug, PartialEq, Eq)]
pub(super) enum LaunchWait {
    /// The guest handshake already completed.
    Ready,
    /// The VM is running and the process answers IPC.
    Launched,
    /// The instance disappeared before either sentinel: a crash.
    Crashed,
    /// Nothing within the ceiling; treated as still booting.
    TimedOut,
}

/// Wait for the launch signal, readiness, or a crash, at most `ceiling`.
/// `still_alive` reports whether the instance is still registered; it is
/// consulted only when neither sentinel exists so a crash after launch
/// (which exec surfaces) does not turn a successful create into an error.
pub(super) async fn wait_for_launch(
    ready_path: &Path,
    launched_path: &Path,
    still_alive: impl Fn() -> bool,
    ceiling: Duration,
) -> LaunchWait {
    let opts = PollOpts {
        label: "provision-launch",
        timeout: ceiling,
        initial_delay: Duration::from_millis(5),
        max_delay: Duration::from_millis(50),
    };
    let outcome = poll_until(opts, || async {
        if ready_path.exists() {
            return Some(LaunchWait::Ready);
        }
        if launched_path.exists() {
            return Some(LaunchWait::Launched);
        }
        (!still_alive()).then_some(LaunchWait::Crashed)
    })
    .await;
    outcome.unwrap_or(LaunchWait::TimedOut)
}

#[cfg(test)]
mod tests;
