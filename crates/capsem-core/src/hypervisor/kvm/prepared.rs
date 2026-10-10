//! Resources that must be acquired before the VM-owner sandbox is installed.

use anyhow::Result;
use tokio::sync::mpsc;

use super::{kvm_vsock_seed, virtio_vsock, KvmHypervisor};
use crate::hypervisor::{Hypervisor, VmHandle, VsockConnection};
use crate::vm::config::VmConfig;

/// AF_VSOCK listeners created before direct socket authority is dropped.
pub(crate) struct PreparedVsock(pub(super) Option<virtio_vsock::BoundVsockListeners>);

impl KvmHypervisor {
    pub(crate) fn prepare_vsock(vsock_ports: &[u32], seed: u32) -> Result<PreparedVsock> {
        let bindings = if vsock_ports.is_empty() {
            None
        } else {
            Some(virtio_vsock::bind_vsock_listeners_for_vm(vsock_ports, seed)?)
        };
        Ok(PreparedVsock(bindings))
    }
}

impl Hypervisor for KvmHypervisor {
    fn boot(
        &self,
        config: &VmConfig,
        vsock_ports: &[u32],
    ) -> Result<(Box<dyn VmHandle>, mpsc::UnboundedReceiver<VsockConnection>)> {
        let prepared = Self::prepare_vsock(vsock_ports, kvm_vsock_seed(config))?;
        self.boot_prepared(config, vsock_ports, prepared)
    }
}
