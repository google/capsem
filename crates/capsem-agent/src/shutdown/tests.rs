use super::*;
use std::process::{Child, Command};

fn kill_hangup_default() -> nix::Result<nix::sys::signal::SigHandler> {
    unsafe { nix::sys::signal::signal(Signal::SIGHUP, nix::sys::signal::SigHandler::SigDfl) }
}

/// Spawn with SIGHUP at its default disposition, as the agent does for its
/// shell: a harness started under `nohup` would otherwise hand the child an
/// ignored SIGHUP and no shell here would ever exit on hangup.
fn spawn(cmd: &str) -> (Child, Pid) {
    use std::os::unix::process::CommandExt;
    let mut command = Command::new("sh");
    command.arg("-c").arg(cmd);
    // SAFETY: only async-signal-safe work before exec, a single signal(2).
    unsafe {
        command.pre_exec(|| {
            let _ = kill_hangup_default();
            Ok(())
        });
    }
    let child = command.spawn().expect("spawn sh");
    let pid = Pid::from_raw(child.id() as i32);
    (child, pid)
}

/// The shell's cooperative path: SIGHUP ends it and we return as soon as it
/// is reaped, not at the end of the grace period.
#[test]
fn hangup_ends_shell_long_before_the_grace_period() {
    let (_child, pid) = spawn("exec sleep 300");
    let grace = Duration::from_secs(5);
    let started = Instant::now();
    let end = hang_up_and_wait(pid, grace);
    let took = started.elapsed();
    assert!(matches!(end, ShellEnd::Exited { .. }), "{end}");
    assert!(
        took < Duration::from_secs(1),
        "waited {took:?} for a shell that exits on SIGHUP"
    );
    // Reaped by us: a second wait has nothing to collect.
    assert_eq!(waitpid(pid, Some(WaitPidFlag::WNOHANG)), Err(nix::errno::Errno::ECHILD));
}

/// A shell that ignores SIGHUP is killed at the grace ceiling, no later.
#[test]
fn shell_ignoring_hangup_is_killed_at_the_grace_ceiling() {
    let (_child, pid) = spawn("trap '' HUP TERM; exec sleep 300");
    // Let sh install the trap before we signal, or the HUP lands first.
    std::thread::sleep(Duration::from_millis(100));
    let grace = Duration::from_millis(300);
    let started = Instant::now();
    let end = hang_up_and_wait(pid, grace);
    let took = started.elapsed();
    assert_eq!(end, ShellEnd::Killed { grace });
    assert!(took >= grace, "killed before the grace period: {took:?}");
    assert!(took < grace + Duration::from_secs(1), "SIGKILL reap took {took:?}");
    assert_eq!(waitpid(pid, Some(WaitPidFlag::WNOHANG)), Err(nix::errno::Errno::ECHILD));
}

/// A shell that already exited (a zombie nobody reaped) is collected at once.
#[test]
fn already_exited_shell_returns_immediately() {
    let (_child, pid) = spawn("exit 0");
    std::thread::sleep(Duration::from_millis(100));
    let started = Instant::now();
    let end = hang_up_and_wait(pid, Duration::from_secs(5));
    assert!(matches!(end, ShellEnd::Exited { .. }), "{end}");
    assert!(started.elapsed() < Duration::from_millis(500));
}

/// A pid that was reaped by someone else must not stall the shutdown.
#[test]
fn already_reaped_shell_returns_immediately() {
    let (mut child, pid) = spawn("exit 0");
    child.wait().expect("reap");
    let started = Instant::now();
    let end = hang_up_and_wait(pid, Duration::from_secs(5));
    assert!(matches!(end, ShellEnd::Exited { .. }), "{end}");
    assert!(started.elapsed() < Duration::from_millis(500));
}

#[test]
fn shell_end_display_reports_the_outcome_and_timing() {
    let exited = ShellEnd::Exited {
        elapsed: Duration::from_millis(12),
    };
    assert_eq!(exited.to_string(), "shell exited after 12ms");
    let killed = ShellEnd::Killed {
        grace: Duration::from_secs(2),
    };
    assert_eq!(killed.to_string(), "shell killed after 2000ms grace");
}

/// The full path adds the disk syncs around the wait; the outcome is the
/// same. Timing is not asserted here because sync(2) on a busy host is
/// measured in seconds.
#[test]
fn end_terminal_shell_syncs_around_the_wait_and_reports_the_shell_end() {
    let (_child, pid) = spawn("exec sleep 300");
    let end = end_terminal_shell(pid, Duration::from_secs(5));
    assert!(matches!(end, ShellEnd::Exited { .. }), "{end}");
    assert_eq!(waitpid(pid, Some(WaitPidFlag::WNOHANG)), Err(nix::errno::Errno::ECHILD));
}

#[test]
fn host_shutdown_wait_returns_as_soon_as_the_report_is_marked() {
    let shutdown = std::sync::Arc::new(HostShutdown::default());
    assert!(!shutdown.is_requested());
    shutdown.request();
    assert!(shutdown.is_requested());
    let writer = std::sync::Arc::clone(&shutdown);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(30));
        writer.mark_reported();
    });
    let started = Instant::now();
    assert!(shutdown.wait_reported(Duration::from_secs(5)));
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn host_shutdown_wait_gives_up_at_the_bound() {
    let shutdown = HostShutdown::default();
    let bound = Duration::from_millis(50);
    let started = Instant::now();
    assert!(!shutdown.wait_reported(bound));
    let took = started.elapsed();
    assert!(took >= bound && took < Duration::from_secs(1), "{took:?}");
}

#[test]
fn host_shutdown_already_reported_does_not_wait() {
    let shutdown = HostShutdown::default();
    shutdown.mark_reported();
    let started = Instant::now();
    assert!(shutdown.wait_reported(Duration::from_secs(5)));
    assert!(started.elapsed() < Duration::from_millis(100));
}
