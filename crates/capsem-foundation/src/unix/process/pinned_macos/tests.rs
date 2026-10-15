use super::*;
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
fn darwin_process_handle_has_fixed_native_identity_layout() {
    assert_eq!(std::mem::size_of::<UniqueInfo>(), 56);
    assert_eq!(std::mem::size_of::<ProcessInfo>(), 192);
    assert_eq!(std::mem::size_of::<AuditToken>(), 32);
}

#[test]
fn darwin_process_handle_signals_only_original_identity_and_observes_exit() {
    let mut child = sleeper();
    let handle = ProcessHandle::open(ProcessId::try_from(child.0.id()).unwrap()).unwrap();
    assert!(!handle.has_exited().unwrap());
    assert!(!handle
        .wait_until_exit(Instant::now() + Duration::from_millis(15))
        .unwrap());
    assert!(child.0.try_wait().unwrap().is_none());
    match handle.signal(Signal::Kill) {
        Ok(outcome) => assert_eq!(outcome, SignalOutcome::Delivered),
        Err(error) if error.kind() == io::ErrorKind::Unsupported => {
            // 14 has no guarded signal API. The owner of this unreaped child
            // can retire it; the table-pinned handle must never use raw kill.
            assert!(child.0.try_wait().unwrap().is_none());
            child.0.kill().unwrap();
        }
        Err(error) => panic!("guarded native signal failed: {error}"),
    }
    assert!(handle.wait_until_exit(Instant::now() + Duration::from_secs(2)).unwrap());
    child.0.wait().unwrap();
    let mut replacement = sleeper();
    assert!(handle.has_exited().unwrap());
    assert_eq!(handle.signal(Signal::Kill).unwrap(), SignalOutcome::Gone);
    assert!(replacement.0.try_wait().unwrap().is_none());
}

#[test]
fn darwin_process_handle_wrong_pid_version_cannot_signal_or_acknowledge_live_child_exit() {
    let mut child = sleeper();
    let mut handle = ProcessHandle::open(ProcessId::try_from(child.0.id()).unwrap()).unwrap();
    handle.token.0[7] ^= 1;
    assert!(handle.signal(Signal::Kill).is_err());
    assert!(!handle.has_exited().unwrap(), "wrong PID version is not process exit");
    assert!(child.0.try_wait().unwrap().is_none());
}

#[test]
fn darwin_process_handle_observes_native_zombie_without_reaping_it() {
    let mut child = Child(std::process::Command::new("false").env_clear().spawn().unwrap());
    let handle = ProcessHandle::open(ProcessId::try_from(child.0.id()).unwrap()).unwrap();
    assert!(handle.wait_until_exit(Instant::now() + Duration::from_secs(2)).unwrap());
    assert_eq!(child.0.wait().unwrap().code(), Some(1));
    assert!(ProcessHandle::open(ProcessId::try_from(i32::MAX as u32).unwrap()).is_err());
}
