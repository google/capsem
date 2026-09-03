use super::{probe, ProcessProbe, ProcessState};

#[test]
fn invalid_pid_is_not_mistaken_for_a_gone_process() {
    assert_eq!(probe(0).unwrap_err().kind(), std::io::ErrorKind::InvalidInput);
    let mut conservative = ProcessProbe::new("test-invalid-pid");
    assert!(conservative.is_alive(0));
    assert!(!conservative.is_gone(0));
}

#[test]
fn real_child_is_observed_alive_then_gone() {
    let mut child = std::process::Command::new("sh")
        .args(["-c", "sleep 30"])
        .spawn()
        .unwrap();
    let pid = child.id();
    let mut probe = ProcessProbe::new("test-real-child");
    assert_eq!(probe.state(pid), ProcessState::Alive);
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(probe.state(pid), ProcessState::Gone);
}
