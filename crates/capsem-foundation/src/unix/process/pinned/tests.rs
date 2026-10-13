use super::*;
use crate::unix::process::{ProcessId, Signal, SignalOutcome};
use std::time::{Duration, Instant};

struct Child(std::process::Child);

impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn sleeper() -> Child {
    Child(
        std::process::Command::new("sleep")
            .arg("30")
            .env_clear()
            .spawn()
            .unwrap(),
    )
}

#[test]
fn pinned_process_wait_is_finite_and_never_signals_a_live_process() {
    use nix::fcntl::{fcntl, FcntlArg, FdFlag};
    use std::os::fd::AsRawFd;

    let mut child = sleeper();
    let handle = ProcessHandle::open(ProcessId::try_from(child.0.id()).unwrap()).unwrap();
    let flags = fcntl(handle.as_raw_fd(), FcntlArg::F_GETFD).unwrap();
    assert!(FdFlag::from_bits_retain(flags).contains(FdFlag::FD_CLOEXEC));
    assert!(!handle.has_exited().unwrap());
    assert!(!handle.wait_until_exit(Instant::now()).unwrap());
    let start = Instant::now();
    assert!(!handle.wait_until_exit(start + Duration::from_millis(15)).unwrap());
    assert!(start.elapsed() < Duration::from_secs(1));
    assert!(child.0.try_wait().unwrap().is_none());
}

#[test]
fn pinned_process_signal_and_exit_are_bound_to_original_native_process() {
    let mut original = sleeper();
    let handle = ProcessHandle::open(ProcessId::try_from(original.0.id()).unwrap()).unwrap();
    assert_eq!(handle.signal(Signal::Kill).unwrap(), SignalOutcome::Delivered);
    assert!(handle.wait_until_exit(Instant::now() + Duration::from_secs(2)).unwrap());
    assert!(!original.0.wait().unwrap().success());
    assert!(handle.has_exited().unwrap());
    let mut replacement = sleeper();
    assert_eq!(handle.signal(Signal::Kill).unwrap(), SignalOutcome::Gone);
    assert_eq!(handle.signal(Signal::Terminate).unwrap(), SignalOutcome::Gone);
    assert!(replacement.0.try_wait().unwrap().is_none());
}

#[test]
fn pinned_process_can_observe_an_unreaped_exit_and_refuses_missing_pid() {
    let mut original = Child(std::process::Command::new("false").env_clear().spawn().unwrap());
    let handle = ProcessHandle::open(ProcessId::try_from(original.0.id()).unwrap()).unwrap();
    assert!(handle.wait_until_exit(Instant::now() + Duration::from_secs(2)).unwrap());
    assert_eq!(original.0.wait().unwrap().code(), Some(1));
    assert!(ProcessHandle::open(ProcessId::try_from(i32::MAX as u32).unwrap()).is_err());
}
