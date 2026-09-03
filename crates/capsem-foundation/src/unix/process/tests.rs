use std::time::Duration;

use nix::errno::Errno;

use super::*;

#[test]
fn process_ids_reject_values_the_kernel_cannot_name() {
    assert!(ProcessId::try_from(0).is_err());
    assert!(ProcessId::try_from(i32::MAX as u32 + 1).is_err());
    assert_eq!(ProcessId::try_from(42).unwrap().get(), 42);
}

#[test]
fn probe_classifies_only_esrch_as_gone() {
    assert_eq!(classify_probe(Ok(())).unwrap(), ProcessState::Alive);
    assert_eq!(classify_probe(Err(Errno::EPERM)).unwrap(), ProcessState::Alive);
    assert_eq!(classify_probe(Err(Errno::ESRCH)).unwrap(), ProcessState::Gone);
    assert_eq!(
        classify_probe(Err(Errno::EINVAL)).unwrap_err().raw_os_error(),
        Some(Errno::EINVAL as i32)
    );
}

#[test]
fn signal_classifies_disappearance_without_hiding_other_errno() {
    assert_eq!(classify_signal(Ok(())).unwrap(), SignalOutcome::Delivered);
    assert_eq!(classify_signal(Err(Errno::ESRCH)).unwrap(), SignalOutcome::Gone);
    assert_eq!(
        classify_signal(Err(Errno::EPERM)).unwrap_err().raw_os_error(),
        Some(Errno::EPERM as i32)
    );
}

#[test]
fn current_identity_is_numeric_and_has_a_parent() {
    let _uid: u32 = current_uid();
    assert!(parent_process_id().is_some());
}

#[test]
fn a_real_child_moves_from_alive_to_gone() {
    let mut child = std::process::Command::new("sleep").arg("30").spawn().unwrap();
    let pid = ProcessId::try_from(child.id()).unwrap();

    assert_eq!(probe(pid).unwrap(), ProcessState::Alive);
    assert_eq!(send_signal(pid, Signal::Kill).unwrap(), SignalOutcome::Delivered);
    child.wait().unwrap();

    for _ in 0..20 {
        if probe(pid).unwrap() == ProcessState::Gone {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("reaped child remained visible to kill(pid, 0)");
}
