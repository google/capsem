//! KVM hypervisor backend for Linux.
//!
//! Direct KVM ioctls with in-process virtio device emulation.
//! No QEMU, no crosvm, no external VMM -- 100% embedded.

mod sys;
mod memory;
mod fdt;
mod boot;
mod mmio;
mod vcpu;
mod virtio_mmio;
mod virtio_queue;
mod virtio_console;
mod virtio_blk;
mod virtio_vsock;
mod virtio_fs;
mod serial;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use tokio::sync::mpsc;

use crate::vm::VmState;
use crate::vm::config::VmConfig;
use super::{Hypervisor, SerialConsole, VmHandle, VsockConnection};

/// KVM hypervisor backend.
pub struct KvmHypervisor;

impl Hypervisor for KvmHypervisor {
    fn boot(
        &self,
        config: &VmConfig,
        vsock_ports: &[u32],
    ) -> Result<(Box<dyn VmHandle>, mpsc::UnboundedReceiver<VsockConnection>)> {
        // Phase 1: Open KVM, create VM, load kernel, boot
        let kvm = sys::KvmFd::open()?;
        let vm = kvm.create_vm()?;

        // Allocate guest memory and register with KVM
        let guest_mem = memory::GuestMemory::new(config.ram_bytes)?;
        vm.set_user_memory_region(
            0,
            memory::RAM_BASE,
            config.ram_bytes,
            guest_mem.as_ptr(),
        )?;

        // Create vCPUs (must happen before GIC init)
        let mut vcpu_fds = Vec::new();
        for i in 0..config.cpu_count {
            let vcpu = vm.create_vcpu(i)?;
            vcpu_fds.push(vcpu);
        }

        // Create and initialize GICv3
        vm.create_gic(config.cpu_count)?;

        // Load kernel into guest memory
        let kernel_info = boot::load_kernel(&guest_mem, &config.kernel_path)?;

        // Load initrd (if present) at end of RAM
        let initrd_info = config
            .initrd_path
            .as_ref()
            .map(|p| boot::load_initrd(&guest_mem, p, config.ram_bytes))
            .transpose()?;

        // Build virtio device list for FDT
        let mut virtio_devices = vec![
            fdt::VirtioDeviceInfo {
                base_addr: memory::virtio_mmio_addr(0),
                irq: memory::virtio_mmio_irq(0),
            },
        ];

        // Add block device slots if configured
        if config.disk_path.is_some() {
            virtio_devices.push(fdt::VirtioDeviceInfo {
                base_addr: memory::virtio_mmio_addr(1),
                irq: memory::virtio_mmio_irq(1),
            });
        }
        if config.scratch_disk_path.is_some() {
            virtio_devices.push(fdt::VirtioDeviceInfo {
                base_addr: memory::virtio_mmio_addr(2),
                irq: memory::virtio_mmio_irq(2),
            });
        }

        // Add vsock device slot (slot 3) if vsock ports requested
        if !vsock_ports.is_empty() {
            virtio_devices.push(fdt::VirtioDeviceInfo {
                base_addr: memory::virtio_mmio_addr(3),
                irq: memory::virtio_mmio_irq(3),
            });
        }

        // Add virtio-fs device slots (slot 4+) for VirtioFS shares
        for (i, _share) in config.virtio_fs_shares.iter().enumerate() {
            let slot = 4 + i as u32;
            virtio_devices.push(fdt::VirtioDeviceInfo {
                base_addr: memory::virtio_mmio_addr(slot),
                irq: memory::virtio_mmio_irq(slot),
            });
        }

        // Generate and load FDT
        let fdt_config = fdt::FdtConfig {
            ram_base: memory::RAM_BASE,
            ram_size: config.ram_bytes,
            cpu_count: config.cpu_count,
            cmdline: config.kernel_cmdline.clone(),
            initrd_start: initrd_info.as_ref().map(|i| i.guest_addr).unwrap_or(0),
            initrd_end: initrd_info
                .as_ref()
                .map(|i| i.guest_addr + i.size as u64)
                .unwrap_or(0),
            virtio_devices,
        };
        let fdt_blob = fdt::build_fdt(&fdt_config)?;
        let fdt_addr = boot::load_fdt(&guest_mem, &fdt_blob, kernel_info.kernel_end)?;

        // Create virtio console + serial console (pipe-backed)
        let (console_device, serial_console) = virtio_console::VirtioConsoleDevice::new()?;
        serial_console.spawn_reader();

        // Build MMIO bus with virtio console
        let mmio_bus = Arc::new(mmio::MmioBus::new());
        let console_mmio = virtio_mmio::VirtioMmioTransport::new(
            Box::new(console_device),
            guest_mem.clone_ref(),
        );
        mmio_bus.register(
            memory::virtio_mmio_addr(0),
            memory::VIRTIO_MMIO_SIZE,
            Arc::new(console_mmio),
        )?;

        // Register block devices on MMIO bus
        if let Some(ref disk_path) = config.disk_path {
            let blk_device = virtio_blk::VirtioBlockDevice::new(disk_path, true)?;
            let blk_mmio = virtio_mmio::VirtioMmioTransport::new(
                Box::new(blk_device),
                guest_mem.clone_ref(),
            );
            mmio_bus.register(
                memory::virtio_mmio_addr(1),
                memory::VIRTIO_MMIO_SIZE,
                Arc::new(blk_mmio),
            )?;
        }

        if let Some(ref scratch_path) = config.scratch_disk_path {
            let scratch_device = virtio_blk::VirtioBlockDevice::new(scratch_path, false)?;
            let scratch_mmio = virtio_mmio::VirtioMmioTransport::new(
                Box::new(scratch_device),
                guest_mem.clone_ref(),
            );
            mmio_bus.register(
                memory::virtio_mmio_addr(2),
                memory::VIRTIO_MMIO_SIZE,
                Arc::new(scratch_mmio),
            )?;
        }

        // Register VirtioFS devices on MMIO bus (slot 4+)
        for (i, share) in config.virtio_fs_shares.iter().enumerate() {
            let slot = 4 + i as u32;

            // Create an eventfd for the worker thread to inject interrupts
            let fs_irq_fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC) };
            anyhow::ensure!(fs_irq_fd >= 0, "failed to create eventfd for VirtioFS");

            // Wire the eventfd to the GIC interrupt line for this slot
            let fs_gsi = memory::virtio_mmio_irq(slot) - 32;
            vm.irqfd(fs_irq_fd, fs_gsi)?;

            let fs_device = virtio_fs::VirtioFsDevice::new(
                &share.tag,
                &share.host_path,
                share.read_only,
                fs_irq_fd,
            )?;
            let fs_mmio = virtio_mmio::VirtioMmioTransport::new(
                Box::new(fs_device),
                guest_mem.clone_ref(),
            );
            mmio_bus.register(
                memory::virtio_mmio_addr(slot),
                memory::VIRTIO_MMIO_SIZE,
                Arc::new(fs_mmio),
            )?;
        }

        // Set initial registers on boot vCPU (vCPU 0)
        boot::set_boot_regs(&vcpu_fds[0], kernel_info.entry_addr, fdt_addr)?;

        // Initialize all vCPUs
        let preferred_target = vm.preferred_target()?;
        for (i, vcpu) in vcpu_fds.iter().enumerate() {
            let power_off = i > 0; // secondary vCPUs start powered off
            vcpu.vcpu_init(&preferred_target, power_off)?;
        }

        // Set up vsock (vhost-vsock + AF_VSOCK listeners)
        let (vsock_tx, vsock_rx) = mpsc::unbounded_channel();
        let shutdown = Arc::new(AtomicBool::new(false));
        let mut vsock_listener_handles = Vec::new();

        if !vsock_ports.is_empty() {
            let guest_cid = 3u32;
            let vhost_fd = virtio_vsock::open_vhost_vsock()?;
            let (vsock_device, call_fds) =
                virtio_vsock::VhostVsockDevice::new(guest_cid, vhost_fd)?;

            // Register vsock on MMIO bus at slot 3
            let vsock_mmio = virtio_mmio::VirtioMmioTransport::new(
                Box::new(vsock_device),
                guest_mem.clone_ref(),
            );
            mmio_bus.register(
                memory::virtio_mmio_addr(3),
                memory::VIRTIO_MMIO_SIZE,
                Arc::new(vsock_mmio),
            )?;

            // Wire call eventfds to GIC IRQ (all 3 vrings share same SPI)
            let vsock_gsi = memory::virtio_mmio_irq(3) - 32;
            for &call_fd in &call_fds {
                vm.irqfd(call_fd, vsock_gsi)?;
            }

            // Spawn AF_VSOCK listeners (before vCPU threads so listeners are
            // ready when the guest agent connects)
            vsock_listener_handles = virtio_vsock::spawn_vsock_listeners(
                guest_cid,
                vsock_ports,
                vsock_tx,
                Arc::clone(&shutdown),
            );
        }

        // Spawn vCPU run loop threads
        let mut vcpu_handles = Vec::new();
        for vcpu in vcpu_fds {
            let handle = vcpu::run_vcpu(
                vcpu,
                Arc::clone(&mmio_bus),
                Arc::clone(&shutdown),
            );
            vcpu_handles.push(handle);
        }

        let handle = KvmHandle {
            state: std::sync::atomic::AtomicU8::new(VmState::Running as u8),
            serial: serial_console,
            shutdown,
            _vcpu_handles: vcpu_handles,
            _guest_mem: guest_mem,
            _mmio_bus: mmio_bus,
            _vsock_listener_handles: vsock_listener_handles,
        };

        Ok((Box::new(handle), vsock_rx))
    }
}

/// A running KVM virtual machine.
struct KvmHandle {
    state: std::sync::atomic::AtomicU8,
    serial: serial::KvmSerialConsole,
    shutdown: Arc<AtomicBool>,
    _vcpu_handles: Vec<std::thread::JoinHandle<Result<()>>>,
    _guest_mem: memory::GuestMemory,
    _mmio_bus: Arc<mmio::MmioBus>,
    _vsock_listener_handles: Vec<std::thread::JoinHandle<()>>,
}

// Safety: all fields are Send, vCPU threads are managed via JoinHandles.
unsafe impl Send for KvmHandle {}

impl VmHandle for KvmHandle {
    fn stop(&self) -> Result<()> {
        self.shutdown.store(true, Ordering::SeqCst);
        self.state.store(VmState::Stopped as u8, Ordering::SeqCst);
        Ok(())
    }

    fn state(&self) -> VmState {
        let val = self.state.load(Ordering::SeqCst);
        if val == VmState::Running as u8 {
            VmState::Running
        } else {
            VmState::Stopped
        }
    }

    fn serial(&self) -> &dyn SerialConsole {
        &self.serial
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Compile-time trait conformance checks
    fn _assert_hypervisor(_: &dyn Hypervisor) {}
    fn _assert_vm_handle(_: &dyn VmHandle) {}
    fn _assert_serial(_: &dyn SerialConsole) {}

    fn _assert_send_sync() {
        fn assert_send<T: Send>() {}
        fn assert_sync<T: Sync>() {}
        assert_send::<KvmHypervisor>();
        assert_sync::<KvmHypervisor>();
        assert_send::<KvmHandle>();
    }

    #[test]
    fn kvm_hypervisor_is_hypervisor() {
        let h = KvmHypervisor;
        _assert_hypervisor(&h);
    }

    #[test]
    fn kvm_hypervisor_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<KvmHypervisor>();
    }

    #[test]
    fn boot_without_kvm_fails_gracefully() {
        // On macOS or without /dev/kvm, boot should fail with an error, not panic
        let h = KvmHypervisor;
        let config = crate::vm::config::VmConfig::builder()
            .kernel_path("/nonexistent/vmlinuz")
            .build();
        assert!(config.is_err());
    }
}
