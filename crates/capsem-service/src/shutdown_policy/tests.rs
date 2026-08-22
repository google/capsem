use super::ShutdownMode;

#[test]
fn discarding_state_does_not_wait_for_a_flush() {
    // Nothing to preserve: the session directory is about to be removed, so a
    // guest sync buys nothing and the budget stays short.
    assert!(!ShutdownMode::Discard.retains_state());
    assert!(ShutdownMode::Discard.exit_timeout().as_secs() <= 2);
}

#[test]
fn retaining_state_waits_long_enough_for_a_slow_disk_to_flush() {
    // The budget exists so the guest can sync before teardown. At five seconds
    // it was sized for an idle developer machine: on a loaded CI runner with
    // several VMs competing for I/O the sync did not finish, the guest was
    // SIGKILLed, and a persistent VM came back missing files it had
    // acknowledged writing -- reported as "first resume lost files written
    // before first stop".
    //
    // A fixed number cannot be right for every disk, so this asserts only that
    // it is not sized for a fast one.
    assert!(ShutdownMode::Retain.retains_state());
    assert!(
        ShutdownMode::Retain.exit_timeout().as_secs() >= 30,
        "a state-retaining stop must outlast a slow guest sync, got {}s",
        ShutdownMode::Retain.exit_timeout().as_secs()
    );
}

#[test]
fn a_retaining_stop_waits_far_longer_than_a_discarding_one() {
    assert!(ShutdownMode::Retain.exit_timeout() > ShutdownMode::Discard.exit_timeout() * 10);
}
