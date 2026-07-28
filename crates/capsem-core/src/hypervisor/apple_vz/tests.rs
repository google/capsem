use super::*;

// -----------------------------------------------------------------------
// Trait implementation checks (compile-time + runtime)
// -----------------------------------------------------------------------

fn _assert_hypervisor(_: &dyn Hypervisor) {}
fn _assert_vm_handle(_: &dyn VmHandle) {}
fn _assert_serial(_: &dyn SerialConsole) {}

#[test]
fn apple_vz_hypervisor_is_hypervisor() {
    let h = AppleVzHypervisor;
    _assert_hypervisor(&h);
}

#[test]
fn apple_vz_hypervisor_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<AppleVzHypervisor>();
}

#[test]
fn apple_vz_handle_is_send() {
    fn assert_send<T: Send>() {}
    assert_send::<AppleVzHandle>();
}

// -----------------------------------------------------------------------
// Serial console trait impl
// -----------------------------------------------------------------------

#[test]
fn serial_console_subscribe_returns_receiver() {
    let (read_fd, _write_fd) = {
        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        (fds[0], fds[1])
    };
    let console = serial::create_console_from_fd(read_fd, -1);
    let trait_ref: &dyn SerialConsole = &console;
    let _rx = trait_ref.subscribe();
}

#[test]
fn serial_console_input_fd_returns_stored_fd() {
    let (read_fd, write_fd) = {
        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        (fds[0], fds[1])
    };
    let console = serial::create_console_from_fd(read_fd, write_fd);
    let trait_ref: &dyn SerialConsole = &console;
    assert_eq!(trait_ref.input_fd(), write_fd);
}

#[test]
fn serial_console_negative_input_fd() {
    let (read_fd, _write_fd) = {
        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        (fds[0], fds[1])
    };
    let console = serial::create_console_from_fd(read_fd, -1);
    let trait_ref: &dyn SerialConsole = &console;
    assert_eq!(trait_ref.input_fd(), -1);
}

// -----------------------------------------------------------------------
// Boot without entitlement (cargo test is unsigned)
// -----------------------------------------------------------------------

#[test]
fn boot_without_assets_fails() {
    let _h = AppleVzHypervisor;
    let config = crate::vm::config::VmConfig::builder()
        .kernel_path("/nonexistent/vmlinuz")
        .build();
    // Should fail at config validation (missing kernel)
    assert!(config.is_err());
}

#[test]
fn boot_with_fake_kernel_fails_gracefully() {
    let tmp = tempfile::tempdir().unwrap();
    let kernel = tmp.path().join("vmlinuz");
    std::fs::write(&kernel, b"not a real kernel").unwrap();

    let config = crate::vm::config::VmConfig::builder()
        .kernel_path(&kernel)
        .build()
        .unwrap();

    let h = AppleVzHypervisor;
    let result = h.boot(&config, &[5000, 5001]);
    // Should fail (no entitlement, or invalid kernel) but not panic
    assert!(result.is_err());
}
