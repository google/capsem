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

#[test]
fn signalling_an_owned_process_group_terminates_its_leader() {
    use std::os::unix::process::{CommandExt, ExitStatusExt};

    let mut child = std::process::Command::new("sleep")
        .arg("30")
        .process_group(0)
        .spawn()
        .unwrap();
    let leader = ProcessId::try_from(child.id()).unwrap();
    assert_eq!(
        send_process_group_signal(leader, Signal::Kill).unwrap(),
        SignalOutcome::Delivered
    );
    assert_eq!(child.wait().unwrap().signal(), Some(nix::libc::SIGKILL));
}

#[test]
fn exit_observation_keeps_the_child_waitable_and_its_pid_reserved() {
    use std::os::unix::process::CommandExt;

    let mut child = std::process::Command::new("sh")
        .args(["-c", "exit 7"])
        .process_group(0)
        .spawn()
        .unwrap();
    let pid = ProcessId::try_from(child.id()).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !child_has_exited(pid).unwrap() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        child_has_exited(pid).unwrap(),
        "observation must not consume the exit status"
    );
    send_process_group_signal(pid, Signal::Kill).expect("an exited child's group needs no signal");
    assert_eq!(child.wait().unwrap().code(), Some(7));
}

#[test]
fn an_exited_group_leader_does_not_hide_a_live_member() {
    use std::os::unix::process::{CommandExt, ExitStatusExt};

    let mut leader = std::process::Command::new("false").process_group(0).spawn().unwrap();
    let pid = ProcessId::try_from(leader.id()).unwrap();
    let mut member = std::process::Command::new("sleep")
        .arg("30")
        .process_group(pid.as_nix().as_raw())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !child_has_exited(pid).unwrap() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    #[cfg(target_os = "macos")]
    let lone = lone_exited_child_group(pid);
    let signalled = send_process_group_signal(pid, Signal::Kill);
    if signalled.is_err() {
        member.kill().unwrap();
    }
    let leader_status = leader.wait().unwrap();
    let member_status = member.wait().unwrap();
    #[cfg(target_os = "macos")]
    assert!(!lone);
    assert_eq!(signalled.unwrap(), SignalOutcome::Delivered);
    assert_eq!(leader_status.code(), Some(1));
    assert_eq!(member_status.signal(), Some(libc::SIGKILL));
}
