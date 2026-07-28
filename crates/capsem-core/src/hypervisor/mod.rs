//! Hypervisor abstraction layer.
//!
//! Defines platform-agnostic traits for VM lifecycle management.
//! Each backend (Apple VZ, KVM, crosvm) implements these traits.

#[cfg(target_os = "macos")]
pub mod apple_vz;

// KVM backend under active development -- allow dead code until device model is complete.
#[cfg(target_os = "linux")]
#[allow(dead_code, unused_imports, unused_variables)]
pub mod kvm;

#[cfg(unix)]
#[allow(dead_code)] // types/constants consumed only by kvm::virtio_fs (linux-gated)
pub(crate) mod fuse;

use std::os::unix::io::RawFd;

use anyhow::Result;
use tokio::sync::{broadcast, mpsc};

use crate::vm::config::VmConfig;
pub use crate::vm::VmState;

/// A hypervisor backend that can boot VMs.
pub trait Hypervisor: Send + Sync {
    /// Boot a VM with the given config and vsock port listeners.
    ///
    /// Returns a handle to the running VM and a channel receiver that
    /// delivers accepted vsock connections from the guest. The receiver
    /// replaces platform-specific vsock manager types -- callers use
    /// `.recv().await` or `.try_recv()` directly.
    fn boot(
        &self,
        config: &VmConfig,
        vsock_ports: &[u32],
    ) -> Result<(Box<dyn VmHandle>, mpsc::UnboundedReceiver<VsockConnection>)>;
}

/// A running VM instance.
///
/// Provides lifecycle control and serial console access.
/// Dropping the handle does NOT stop the VM -- call `stop()` explicitly.
pub trait VmHandle: Send {
    /// Stop the VM.
    fn stop(&self) -> Result<()>;

    /// Get the current VM state.
    fn state(&self) -> VmState;

    /// Access the serial console for boot log streaming and input.
    fn serial(&self) -> &dyn SerialConsole;

    /// Downcast to the concrete backend type.
    fn as_any(&self) -> &dyn std::any::Any;

    /// Pause the VM.
    fn pause(&self) -> Result<()> {
        Err(anyhow::anyhow!(
            "pause not supported by this hypervisor backend"
        ))
    }

    /// Resume the paused VM.
    fn resume(&self) -> Result<()> {
        Err(anyhow::anyhow!(
            "resume not supported by this hypervisor backend"
        ))
    }

    /// Save the VM state to the given path.
    fn save_state(&self, _path: &std::path::Path) -> Result<()> {
        Err(anyhow::anyhow!(
            "save_state not supported by this hypervisor backend"
        ))
    }

    /// Returns true if this hypervisor supports suspend/resume functionality.
    ///
    /// Restore is a boot-time operation: pass `checkpoint_path` to
    /// `Hypervisor::boot()` / `AppleVzMachine::start()`. There is no
    /// post-start `restore_state()` because Apple VZ requires calling
    /// `restoreMachineStateFromURL` before the VM has ever been started.
    fn supports_checkpoint(&self) -> bool {
        false
    }
}

/// Serial console I/O.
pub trait SerialConsole: Send + Sync {
    /// Subscribe to serial output bytes (boot logs).
    fn subscribe(&self) -> broadcast::Receiver<Vec<u8>>;

    /// Raw fd for writing input to the guest serial console.
    fn input_fd(&self) -> RawFd;
}

/// An accepted vsock connection from the guest.
///
/// The `fd` is a valid unix file descriptor for the connection.
/// The internal lifetime anchor keeps platform-specific resources alive
/// so the fd remains valid until this struct is dropped.
pub struct VsockConnection {
    pub fd: RawFd,
    pub port: u32,
    _lifetime_anchor: Box<dyn Send>,
}

impl VsockConnection {
    /// Create a new VsockConnection with a platform-specific lifetime anchor.
    pub fn new(fd: RawFd, port: u32, anchor: Box<dyn Send>) -> Self {
        Self {
            fd,
            port,
            _lifetime_anchor: anchor,
        }
    }
}

// Safety: fd is a valid unix file descriptor usable across threads.
unsafe impl Sync for VsockConnection {}

#[cfg(test)]
mod tests;
