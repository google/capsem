//! Apple Virtualization.framework backend.

pub(crate) mod boot;
pub(crate) mod machine;
pub(crate) mod serial;
pub(crate) mod vsock;

use anyhow::Result;
use objc2::rc::Retained;
use objc2_virtualization::{VZVirtioSocketListener, VZVirtualMachine as ObjcVZVirtualMachine};
use tokio::sync::mpsc;

use crate::vm::VmState;
use crate::vm::config::VmConfig;
use super::{Hypervisor, SerialConsole, VmHandle, VsockConnection};

pub use machine::is_main_thread;

/// Apple Virtualization.framework hypervisor backend.
pub struct AppleVzHypervisor;

impl Hypervisor for AppleVzHypervisor {
    fn boot(
        &self,
        config: &VmConfig,
        vsock_ports: &[u32],
    ) -> Result<(Box<dyn VmHandle>, mpsc::UnboundedReceiver<VsockConnection>)> {
        // Create the VM (configures devices, validates)
        let (machine, serial_console) = machine::AppleVzMachine::create(config)?;

        // Start the VM (spawns serial reader, waits for completion)
        machine.start(&serial_console, config.checkpoint_path.as_deref())?;

        // Set up vsock listeners on the socket device
        let socket_devices = machine.socket_devices();
        let (vsock_rx, delegate, listeners) =
            vsock::setup_vsock_listeners(&socket_devices, vsock_ports)?;

        let handle = AppleVzHandle {
            machine,
            serial: serial_console,
            _vsock_delegate: delegate,
            _vsock_listeners: listeners,
        };

        Ok((Box::new(handle), vsock_rx))
    }
}

/// A running Apple VZ virtual machine.
pub struct AppleVzHandle {
    machine: machine::AppleVzMachine,
    serial: serial::AppleVzSerialConsole,
    // Keep vsock ObjC objects alive so listeners remain active.
    _vsock_delegate: Retained<vsock::VsockListenerDelegate>,
    _vsock_listeners: Vec<Retained<VZVirtioSocketListener>>,
}

// Safety: We manage thread safety through channels and main-thread dispatch.
unsafe impl Send for AppleVzHandle {}

impl VmHandle for AppleVzHandle {
    fn stop(&self) -> Result<()> {
        self.machine.stop()
    }

    fn state(&self) -> VmState {
        self.machine.state()
    }

    fn serial(&self) -> &dyn SerialConsole {
        &self.serial
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn pause(&self) -> Result<()> {
        self.machine.pause()
    }

    fn resume(&self) -> Result<()> {
        self.machine.resume()
    }

    #[cfg(target_os = "macos")]
    fn save_state(&self, path: &std::path::Path) -> Result<()> {
        self.machine.save_state(path)
    }

    #[cfg(target_os = "macos")]
    fn restore_state(&self, path: &std::path::Path) -> Result<()> {
        self.machine.restore_state(path)
    }

    fn supports_checkpoint(&self) -> bool {
        self.machine.supports_checkpoint()
    }
}

impl AppleVzHandle {
    /// Access the underlying VZVirtualMachine for embedding in a VZVirtualMachineView.
    pub fn inner_vz(&self) -> &ObjcVZVirtualMachine {
        self.machine.inner_vz()
    }
}

#[cfg(test)]
mod tests {
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
}
