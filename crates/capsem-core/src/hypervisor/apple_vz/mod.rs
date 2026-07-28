//! Apple Virtualization.framework backend.

pub(crate) mod boot;
pub(crate) mod machine;
pub(crate) mod serial;
pub(crate) mod vsock;

use anyhow::Result;
use objc2::rc::Retained;
use objc2_virtualization::{VZVirtioSocketListener, VZVirtualMachine as ObjcVZVirtualMachine};
use tokio::sync::mpsc;

use super::{Hypervisor, SerialConsole, VmHandle, VsockConnection};
use crate::vm::config::VmConfig;
use crate::vm::VmState;

pub use machine::{is_main_thread, run_on_main_thread};

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

        // Attach the serial-log file writer BEFORE machine.start() spawns the
        // reader thread. tokio::broadcast drops messages when no receiver is
        // attached, and the capsem-process tokio subscriber doesn't attach
        // until after boot_vm() returns -- during restore, that window drops
        // ~100ms of guest output (the reconnect handshake). Subscribing here
        // closes the race.
        if let Some(log_path) = config.serial_log_path.as_deref() {
            use crate::hypervisor::SerialConsole;
            use std::io::Write;
            let mut rx = SerialConsole::subscribe(&serial_console);
            let path = log_path.to_path_buf();
            std::thread::spawn(move || {
                let mut file = match std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                {
                    Ok(f) => f,
                    Err(e) => {
                        tracing::warn!(error = %e, path = %path.display(), "failed to open serial log file");
                        return;
                    }
                };
                loop {
                    match rx.blocking_recv() {
                        Ok(bytes) => {
                            let _ = file.write_all(&bytes);
                            let _ = file.flush();
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            });
        }

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
mod tests;
