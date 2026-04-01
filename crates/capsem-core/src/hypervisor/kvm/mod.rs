//! KVM hypervisor backend for Linux.
//!
//! Direct KVM ioctls with in-process virtio device emulation.
//! No QEMU, no crosvm, no external VMM -- 100% embedded.

mod sys;
mod memory;
#[cfg(target_arch = "aarch64")]
mod fdt;
#[cfg(target_arch = "aarch64")]
mod boot;
#[cfg(target_arch = "x86_64")]
mod boot_x86_64;
mod mmio;
#[cfg(target_arch = "x86_64")]
mod pio;
#[cfg(target_arch = "x86_64")]
mod serial_pio;
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

/// Convert a virtio MMIO IRQ number to a KVM GSI.
/// On aarch64, GIC SPIs start at 32, so we subtract 32 to get the GSI.
/// On x86_64, the IRQ number IS the GSI directly.
fn irq_to_gsi(irq: u32) -> u32 {
    #[cfg(target_arch = "aarch64")]
    { irq - 32 }
    #[cfg(target_arch = "x86_64")]
    { irq }
}

impl Hypervisor for KvmHypervisor {
    fn boot(
        &self,
        config: &VmConfig,
        vsock_ports: &[u32],
    ) -> Result<(Box<dyn VmHandle>, mpsc::UnboundedReceiver<VsockConnection>)> {
        // -- Shared: open KVM, create VM, allocate memory -----------------
        let kvm = sys::KvmFd::open()?;
        let vm = kvm.create_vm()?;

        let guest_mem = memory::GuestMemory::new(config.ram_bytes)?;
        vm.set_user_memory_region(
            0,
            memory::RAM_BASE,
            config.ram_bytes,
            guest_mem.as_ptr(),
        )?;

        // -- Arch-specific: interrupt controller --------------------------
        #[cfg(target_arch = "x86_64")]
        {
            vm.set_tss_addr(0xFFFB_D000)?;
            vm.set_identity_map_addr(0xFFFB_C000)?;
            vm.create_irqchip()?;
            vm.create_pit2()?;
        }

        // Create vCPUs (must happen before GIC init on aarch64)
        let mut vcpu_fds = Vec::new();
        for i in 0..config.cpu_count {
            vcpu_fds.push(vm.create_vcpu(i)?);
        }

        #[cfg(target_arch = "aarch64")]
        vm.create_gic(config.cpu_count)?;

        // -- Arch-specific: kernel loading --------------------------------
        #[cfg(target_arch = "aarch64")]
        let kernel_info = boot::load_kernel(&guest_mem, &config.kernel_path)?;

        #[cfg(target_arch = "x86_64")]
        let kernel_info = boot_x86_64::load_kernel(&guest_mem, &config.kernel_path)?;

        // -- Arch-specific: initrd loading --------------------------------
        #[cfg(target_arch = "aarch64")]
        let initrd_info = config
            .initrd_path
            .as_ref()
            .map(|p| boot::load_initrd(&guest_mem, p, kernel_info.kernel_end))
            .transpose()?;

        #[cfg(target_arch = "x86_64")]
        let initrd_info = config
            .initrd_path
            .as_ref()
            .map(|p| boot_x86_64::load_initrd(&guest_mem, p, kernel_info.kernel_end))
            .transpose()?;

        // -- Arch-specific: FDT (aarch64) / boot_params (x86_64) ---------
        #[cfg(target_arch = "aarch64")]
        {
            let mut virtio_devices = vec![
                fdt::VirtioDeviceInfo {
                    base_addr: memory::virtio_mmio_addr(0),
                    irq: memory::virtio_mmio_irq(0),
                },
            ];
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
            if !vsock_ports.is_empty() {
                virtio_devices.push(fdt::VirtioDeviceInfo {
                    base_addr: memory::virtio_mmio_addr(3),
                    irq: memory::virtio_mmio_irq(3),
                });
            }
            for (i, _share) in config.virtio_fs_shares.iter().enumerate() {
                let slot = 4 + i as u32;
                virtio_devices.push(fdt::VirtioDeviceInfo {
                    base_addr: memory::virtio_mmio_addr(slot),
                    irq: memory::virtio_mmio_irq(slot),
                });
            }
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
            boot::set_boot_regs(&vcpu_fds[0], kernel_info.entry_addr, fdt_addr)?;
        }

        #[cfg(target_arch = "x86_64")]
        {
            // Count virtio MMIO devices for cmdline generation
            let mut device_count: u32 = 1; // console at slot 0
            if config.disk_path.is_some() { device_count += 1; }
            if config.scratch_disk_path.is_some() { device_count += 1; }
            if !vsock_ports.is_empty() { device_count += 1; }
            device_count += config.virtio_fs_shares.len() as u32;

            let cmdline = boot_x86_64::build_cmdline(
                &config.kernel_cmdline,
                device_count,
            );
            let e820 = memory::build_e820_map(config.ram_bytes);

            boot_x86_64::write_gdt(&guest_mem)?;
            boot_x86_64::write_page_tables(&guest_mem, config.ram_bytes)?;
            boot_x86_64::write_boot_params(
                &guest_mem,
                &cmdline,
                initrd_info.as_ref(),
                &e820,
                &kernel_info.setup_header,
            )?;
            boot_x86_64::setup_cpuid(&vm, &vcpu_fds[0])?;
            boot_x86_64::setup_boot_regs(
                &vcpu_fds[0],
                kernel_info.entry_addr,
                memory::BOOT_PARAMS_ADDR,
            )?;
        }

        // -- Arch-specific: vCPU initialization ---------------------------
        #[cfg(target_arch = "aarch64")]
        {
            let preferred_target = vm.preferred_target()?;
            for (i, vcpu) in vcpu_fds.iter().enumerate() {
                let power_off = i > 0;
                vcpu.vcpu_init(&preferred_target, power_off)?;
            }
        }

        #[cfg(target_arch = "x86_64")]
        {
            // CPUID must be set on all vCPUs
            for vcpu in vcpu_fds.iter().skip(1) {
                boot_x86_64::setup_cpuid(&vm, vcpu)?;
            }
        }

        // -- Shared: serial console + MMIO bus ----------------------------
        // On aarch64: virtio-console at slot 0 IS the serial console.
        // On x86_64: virtio-console at slot 0 exists but the primary serial
        //            console is the 16550 UART on PIO 0x3F8.
        let (console_device, serial_console) = virtio_console::VirtioConsoleDevice::new()?;

        #[cfg(target_arch = "x86_64")]
        let (serial_console, uart_output_write, uart_input_read) = {
            // On x86_64, create separate pipes for the 16550 UART and use those
            // for the serial console (boot output goes through ttyS0, not hvc0).
            let (output_read, output_write) = {
                let mut fds = [0i32; 2];
                anyhow::ensure!(unsafe { libc::pipe(fds.as_mut_ptr()) } == 0, "pipe() failed");
                (fds[0], fds[1])
            };
            let (input_read, input_write) = {
                let mut fds = [0i32; 2];
                anyhow::ensure!(unsafe { libc::pipe(fds.as_mut_ptr()) } == 0, "pipe() failed");
                (fds[0], fds[1])
            };
            (
                serial::KvmSerialConsole::new(output_read, input_write),
                output_write,
                input_read,
            )
        };

        serial_console.spawn_reader();

        let mmio_bus = Arc::new(mmio::MmioBus::new());
        let console_mmio = virtio_mmio::VirtioMmioTransport::new(
            Box::new(console_device),
            guest_mem.clone_ref(memory::RAM_BASE),
        );
        mmio_bus.register(
            memory::virtio_mmio_addr(0),
            memory::VIRTIO_MMIO_SIZE,
            Arc::new(console_mmio),
        )?;

        // -- x86_64: PIO bus + 16550 UART ---------------------------------
        #[cfg(target_arch = "x86_64")]
        let pio_bus = {
            let bus = Arc::new(pio::PioBus::new());
            let uart = serial_pio::Serial16550::new(uart_output_write, uart_input_read);
            bus.register(0x3F8, 8, Arc::new(uart))?;
            bus
        };

        // -- Shared: block devices ----------------------------------------
        if let Some(ref disk_path) = config.disk_path {
            let blk_device = virtio_blk::VirtioBlockDevice::new(disk_path, true)?;
            let blk_mmio = virtio_mmio::VirtioMmioTransport::new(
                Box::new(blk_device),
                guest_mem.clone_ref(memory::RAM_BASE),
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
                guest_mem.clone_ref(memory::RAM_BASE),
            );
            mmio_bus.register(
                memory::virtio_mmio_addr(2),
                memory::VIRTIO_MMIO_SIZE,
                Arc::new(scratch_mmio),
            )?;
        }

        // -- Shared: VirtioFS (slot 4+) -----------------------------------
        for (i, share) in config.virtio_fs_shares.iter().enumerate() {
            let slot = 4 + i as u32;
            let fs_irq_fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC) };
            anyhow::ensure!(fs_irq_fd >= 0, "failed to create eventfd for VirtioFS");

            let fs_gsi = irq_to_gsi(memory::virtio_mmio_irq(slot));
            vm.irqfd(fs_irq_fd, fs_gsi)?;

            let fs_device = virtio_fs::VirtioFsDevice::new(
                &share.tag,
                &share.host_path,
                share.read_only,
                fs_irq_fd,
            )?;
            let fs_mmio = virtio_mmio::VirtioMmioTransport::new(
                Box::new(fs_device),
                guest_mem.clone_ref(memory::RAM_BASE),
            );
            mmio_bus.register(
                memory::virtio_mmio_addr(slot),
                memory::VIRTIO_MMIO_SIZE,
                Arc::new(fs_mmio),
            )?;
        }

        // -- Shared: vsock ------------------------------------------------
        let (vsock_tx, vsock_rx) = mpsc::unbounded_channel();
        let shutdown = Arc::new(AtomicBool::new(false));
        let mut vsock_listener_handles = Vec::new();

        if !vsock_ports.is_empty() {
            let guest_cid = 3u32;
            let vhost_fd = virtio_vsock::open_vhost_vsock()?;
            let (vsock_device, call_fds) =
                virtio_vsock::VhostVsockDevice::new(guest_cid, vhost_fd)?;

            let vsock_mmio = virtio_mmio::VirtioMmioTransport::new(
                Box::new(vsock_device),
                guest_mem.clone_ref(memory::RAM_BASE),
            );
            mmio_bus.register(
                memory::virtio_mmio_addr(3),
                memory::VIRTIO_MMIO_SIZE,
                Arc::new(vsock_mmio),
            )?;

            let vsock_gsi = irq_to_gsi(memory::virtio_mmio_irq(3));
            for &call_fd in &call_fds {
                vm.irqfd(call_fd, vsock_gsi)?;
            }

            vsock_listener_handles = virtio_vsock::spawn_vsock_listeners(
                guest_cid,
                vsock_ports,
                vsock_tx,
                Arc::clone(&shutdown),
            );
        }

        // -- Shared: spawn vCPU threads -----------------------------------
        let mut vcpu_handles = Vec::new();
        for vcpu in vcpu_fds {
            let handle = vcpu::run_vcpu(
                vcpu,
                Arc::clone(&mmio_bus),
                #[cfg(target_arch = "x86_64")]
                Arc::clone(&pio_bus),
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
