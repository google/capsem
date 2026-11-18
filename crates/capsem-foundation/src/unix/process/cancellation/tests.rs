use super::*;

#[test]
fn cancellation_capture_is_exclusive_first_signal_wins_and_drop_restores() {
    const CHILD: &str = "CAPSEM_CANCELLATION_GUARD_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "unix::process::cancellation::tests::cancellation_capture_is_exclusive_first_signal_wins_and_drop_restores"])
            .env(CHILD, "1").status().unwrap();
        assert!(status.success());
        return;
    }
    unsafe { signal(Signal::Terminate.as_nix(), SigHandler::SigIgn) }.unwrap();
    let guard = Guard::install().unwrap();
    assert!(Guard::install().is_err());
    assert_eq!(guard.received(), None);
    nix::sys::signal::raise(Signal::Terminate.as_nix()).unwrap();
    nix::sys::signal::raise(Signal::Interrupt.as_nix()).unwrap();
    assert_eq!(guard.received(), Some(Signal::Terminate));
    drop(guard);
    let restored = unsafe { signal(Signal::Terminate.as_nix(), SigHandler::SigDfl) }.unwrap();
    assert!(matches!(restored, SigHandler::SigIgn));
    let guard = Guard::install().unwrap();
    assert_eq!(guard.received(), None);
    drop(guard);
}
