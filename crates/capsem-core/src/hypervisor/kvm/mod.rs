//! KVM hypervisor backend for Linux.
//!
//! Direct KVM ioctls with in-process virtio device emulation.
//! No QEMU, no crosvm, no external VMM -- 100% embedded.

#[cfg(target_arch = "aarch64")]
mod boot;
#[cfg(target_arch = "x86_64")]
mod boot_x86_64;
mod checkpoint;
#[cfg(target_arch = "aarch64")]
mod fdt;
mod memory;
mod mmio;
#[cfg(target_arch = "x86_64")]
mod pio;
mod serial;
#[cfg(target_arch = "x86_64")]
mod serial_pio;
mod sys;
mod vcpu;
mod virtio_blk;
mod virtio_console;
mod virtio_fs;
mod virtio_mmio;
mod virtio_pmem;
mod virtio_queue;
mod virtio_vsock;

use std::fs::File;
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use capsem_proto::metrics::{VmBlockMetrics, VmHypervisorMetrics};
use tokio::sync::mpsc;

use super::{Hypervisor, SerialConsole, VmHandle, VsockConnection};
use crate::vm::config::VmConfig;
use crate::vm::VmState;

const KVM_PAUSE_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(target_arch = "x86_64")]
const KVM_PMEM_MEMORY_SLOT: u32 = 10;
#[cfg(target_arch = "x86_64")]
const KVM_PMEM_ALIGNMENT: u64 = 128 * 1024 * 1024;

fn kvm_vsock_seed(config: &VmConfig) -> u32 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(config.kernel_path.to_string_lossy().as_bytes());
    if let Some(path) = config
        .scratch_disk_path
        .as_ref()
        .or(config.disk_path.as_ref())
    {
        hasher.update(path.to_string_lossy().as_bytes());
    }
    for share in &config.virtio_fs_shares {
        hasher.update(share.tag.as_bytes());
        hasher.update(share.host_path.to_string_lossy().as_bytes());
    }
    let hash = hasher.finalize();
    let mut bytes = [0u8; 4];
    bytes.copy_from_slice(&hash.as_bytes()[..4]);
    u32::from_le_bytes(bytes)
}

fn append_kvm_vsock_port_offset(cmdline: &str, offset: u32) -> String {
    if offset == 0 {
        return cmdline.to_string();
    }
    format!("{cmdline} capsem.vsock_port_offset={offset}")
}

fn create_irq_eventfd() -> Result<OwnedFd> {
    let fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
    anyhow::ensure!(
        fd >= 0,
        "failed to create virtio-mmio IRQ eventfd: {}",
        std::io::Error::last_os_error()
    );
    // Safety: fd was just returned by eventfd and is uniquely owned here.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

#[cfg(target_arch = "x86_64")]
fn create_notify_eventfd() -> Result<OwnedFd> {
    let fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC) };
    anyhow::ensure!(
        fd >= 0,
        "failed to create virtio-mmio notify eventfd: {}",
        std::io::Error::last_os_error()
    );
    // Safety: fd was just returned by eventfd and is uniquely owned here.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// KVM hypervisor backend.
pub struct KvmHypervisor;

/// Convert a virtio MMIO IRQ number to a KVM GSI.
/// On aarch64, GIC SPIs start at 32, so we subtract 32 to get the GSI.
/// On x86_64, the IRQ number IS the GSI directly.
fn irq_to_gsi(irq: u32) -> u32 {
    #[cfg(target_arch = "aarch64")]
    {
        irq - 32
    }
    #[cfg(target_arch = "x86_64")]
    {
        irq
    }
}

#[cfg(target_arch = "x86_64")]
fn should_attach_pmem_rootfs(config: &VmConfig) -> bool {
    config.disk_path.is_some() && env_truthy("CAPSEM_KVM_ROOTFS_PMEM_DAX")
}

#[cfg(target_arch = "x86_64")]
fn should_use_file_backed_pmem_rootfs() -> bool {
    env_truthy("CAPSEM_KVM_ROOTFS_PMEM_FILE_BACKED")
}

#[cfg(target_arch = "x86_64")]
fn env_truthy(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

#[cfg(target_arch = "x86_64")]
fn align_up_to(value: u64, alignment: u64) -> u64 {
    debug_assert!(alignment.is_power_of_two());
    (value + alignment - 1) & !(alignment - 1)
}

#[cfg(target_arch = "x86_64")]
fn pmem_guest_phys_addr(ram_bytes: u64) -> u64 {
    align_up_to(memory::guest_phys_end(ram_bytes), KVM_PMEM_ALIGNMENT)
}

#[cfg(target_arch = "x86_64")]
fn pmem_region_size(image_size: u64) -> u64 {
    align_up_to(image_size, KVM_PMEM_ALIGNMENT)
}

#[cfg(target_arch = "x86_64")]
fn is_pmem_file_backing_size(image_size: u64) -> bool {
    image_size > 0 && image_size == pmem_region_size(image_size)
}

#[cfg(target_arch = "x86_64")]
fn virtio_mmio_device_count(config: &VmConfig, vsock_ports: &[u32]) -> u32 {
    let mut device_count = 1; // console at slot 0
    if config.disk_path.is_some() {
        device_count += 1;
    }
    if config.scratch_disk_path.is_some() {
        device_count += 1;
    }
    if !vsock_ports.is_empty() {
        device_count += 1;
    }
    device_count += config.virtio_fs_shares.len() as u32;
    if should_attach_pmem_rootfs(config) {
        device_count += 1;
    }
    device_count
}

#[cfg(target_arch = "x86_64")]
struct PmemMapping {
    ptr: *mut u8,
    size: u64,
    image_size: u64,
    guest_phys_addr: u64,
    backing: PmemBacking,
    advice: PmemMadvise,
    populate: bool,
}

#[cfg(target_arch = "x86_64")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PmemBacking {
    AnonymousCopy,
    FileMmap,
}

#[cfg(target_arch = "x86_64")]
impl PmemBacking {
    fn as_str(self) -> &'static str {
        match self {
            Self::AnonymousCopy => "anonymous_copy",
            Self::FileMmap => "file_mmap",
        }
    }
}

#[cfg(target_arch = "x86_64")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PmemMadvise {
    None,
    Sequential,
    Random,
    WillNeed,
}

#[cfg(target_arch = "x86_64")]
impl PmemMadvise {
    fn from_env() -> Result<Self> {
        let raw =
            std::env::var("CAPSEM_KVM_ROOTFS_PMEM_MADVISE").unwrap_or_else(|_| "none".to_string());
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "none" => Ok(Self::None),
            "sequential" => Ok(Self::Sequential),
            "random" => Ok(Self::Random),
            "willneed" | "will_need" | "will-need" => Ok(Self::WillNeed),
            other => anyhow::bail!(
                "invalid CAPSEM_KVM_ROOTFS_PMEM_MADVISE={other}; expected none,sequential,random,willneed"
            ),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Sequential => "sequential",
            Self::Random => "random",
            Self::WillNeed => "willneed",
        }
    }

    fn libc_advice(self) -> Option<libc::c_int> {
        match self {
            Self::None => None,
            Self::Sequential => Some(libc::MADV_SEQUENTIAL),
            Self::Random => Some(libc::MADV_RANDOM),
            Self::WillNeed => Some(libc::MADV_WILLNEED),
        }
    }
}

#[cfg(target_arch = "x86_64")]
impl PmemMapping {
    fn from_file(path: &Path, guest_phys_addr: u64) -> Result<Self> {
        let mut file = File::open(path)
            .with_context(|| format!("failed to open pmem rootfs image {}", path.display()))?;
        let image_size = file
            .metadata()
            .with_context(|| format!("failed to stat pmem rootfs image {}", path.display()))?
            .len();
        anyhow::ensure!(
            image_size > 0,
            "pmem rootfs image is empty: {}",
            path.display()
        );
        let size = pmem_region_size(image_size);
        if should_use_file_backed_pmem_rootfs() {
            anyhow::ensure!(
                is_pmem_file_backing_size(image_size),
                "file-backed pmem rootfs image {} must be padded to {} bytes alignment; size={} aligned_size={}",
                path.display(),
                KVM_PMEM_ALIGNMENT,
                image_size,
                size
            );
            return Self::map_file(path, &file, image_size, guest_phys_addr);
        }
        Self::map_anonymous_copy(path, &mut file, image_size, size, guest_phys_addr)
    }

    fn map_file(path: &Path, file: &File, image_size: u64, guest_phys_addr: u64) -> Result<Self> {
        let advice = PmemMadvise::from_env()?;
        let populate = env_truthy("CAPSEM_KVM_ROOTFS_PMEM_POPULATE");
        let mut flags = libc::MAP_PRIVATE;
        if populate {
            flags |= libc::MAP_POPULATE;
        }
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                image_size as usize,
                libc::PROT_READ | libc::PROT_WRITE,
                flags,
                file.as_raw_fd(),
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            anyhow::bail!(
                "failed to mmap file-backed pmem rootfs image {}: {}",
                path.display(),
                std::io::Error::last_os_error()
            );
        }
        if let Some(libc_advice) = advice.libc_advice() {
            let ret = unsafe { libc::madvise(ptr, image_size as usize, libc_advice) };
            if ret != 0 {
                let err = std::io::Error::last_os_error();
                unsafe {
                    libc::munmap(ptr, image_size as usize);
                }
                anyhow::bail!(
                    "failed to madvise file-backed pmem rootfs image {} with {}: {err}",
                    path.display(),
                    advice.as_str()
                );
            }
        }
        Ok(Self {
            ptr: ptr as *mut u8,
            size: image_size,
            image_size,
            guest_phys_addr,
            backing: PmemBacking::FileMmap,
            advice,
            populate,
        })
    }

    fn map_anonymous_copy(
        path: &Path,
        file: &mut File,
        image_size: u64,
        size: u64,
        guest_phys_addr: u64,
    ) -> Result<Self> {
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                size as usize,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            anyhow::bail!(
                "failed to allocate pmem rootfs region: {}",
                std::io::Error::last_os_error()
            );
        }

        let mapping = Self {
            ptr: ptr as *mut u8,
            size,
            image_size,
            guest_phys_addr,
            backing: PmemBacking::AnonymousCopy,
            advice: PmemMadvise::None,
            populate: false,
        };
        let dst = unsafe { std::slice::from_raw_parts_mut(mapping.ptr, image_size as usize) };
        if let Err(err) = file.read_exact(dst) {
            unsafe {
                libc::munmap(mapping.ptr as *mut libc::c_void, mapping.size as usize);
            }
            anyhow::bail!(
                "failed to populate pmem rootfs image {}: {err}",
                path.display()
            );
        }
        Ok(mapping)
    }
}

#[cfg(target_arch = "x86_64")]
impl Drop for PmemMapping {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.ptr as *mut libc::c_void, self.size as usize);
        }
    }
}

impl Hypervisor for KvmHypervisor {
    fn boot(
        &self,
        config: &VmConfig,
        vsock_ports: &[u32],
    ) -> Result<(Box<dyn VmHandle>, mpsc::UnboundedReceiver<VsockConnection>)> {
        #[cfg(not(target_arch = "x86_64"))]
        if config.checkpoint_path.is_some() {
            anyhow::bail!(
                "KVM checkpoint restore is only implemented for x86_64; refusing to ignore checkpoint_path"
            );
        }

        // -- Shared: open KVM, create VM, allocate memory -----------------
        let kvm = sys::KvmFd::open()?;
        let vm = kvm.create_vm()?;

        let guest_mem = memory::GuestMemory::new(config.ram_bytes)?;
        #[cfg(target_arch = "x86_64")]
        for region in memory::kvm_memory_regions(config.ram_bytes) {
            vm.set_user_memory_region(
                region.slot,
                region.guest_phys_addr,
                region.memory_size,
                guest_mem.as_ptr_at(region.host_offset)?,
            )?;
        }
        #[cfg(not(target_arch = "x86_64"))]
        vm.set_user_memory_region(0, memory::RAM_BASE, config.ram_bytes, guest_mem.as_ptr())?;

        #[cfg(target_arch = "x86_64")]
        let restoring = config.checkpoint_path.is_some();

        let vsock_bindings = if vsock_ports.is_empty() {
            None
        } else {
            Some(virtio_vsock::bind_vsock_listeners_for_vm(
                vsock_ports,
                kvm_vsock_seed(config),
            )?)
        };
        let kernel_cmdline = append_kvm_vsock_port_offset(
            &config.kernel_cmdline,
            vsock_bindings.as_ref().map_or(0, |b| b.offset()),
        );
        #[cfg(target_arch = "x86_64")]
        let pmem_mapping = if should_attach_pmem_rootfs(config) {
            let disk_path = config
                .disk_path
                .as_ref()
                .expect("should_attach_pmem_rootfs requires disk_path");
            let guest_phys_addr = pmem_guest_phys_addr(config.ram_bytes);
            let mapping = PmemMapping::from_file(disk_path, guest_phys_addr)?;
            vm.set_user_memory_region(
                KVM_PMEM_MEMORY_SLOT,
                mapping.guest_phys_addr,
                mapping.size,
                mapping.ptr,
            )?;
            tracing::info!(
                event_name = "kvm.pmem_rootfs.attach",
                path = %disk_path.display(),
                guest_phys_addr = format_args!("{:#x}", mapping.guest_phys_addr),
                size = mapping.size,
                image_size = mapping.image_size,
                backing = mapping.backing.as_str(),
                madvise = mapping.advice.as_str(),
                populate = mapping.populate,
                "attached read-only rootfs image as virtio-pmem backing memory"
            );
            Some(mapping)
        } else {
            None
        };

        // -- Arch-specific: interrupt controller --------------------------
        #[cfg(target_arch = "x86_64")]
        let has_pit = {
            vm.set_tss_addr(0xFFFB_D000)?;
            vm.set_identity_map_addr(0xFFFB_C000)?;
            match vm.create_irqchip() {
                Ok(()) => {
                    tracing::info!("KVM full IRQCHIP enabled");
                    match vm.create_pit2() {
                        Ok(()) => true,
                        Err(e) => {
                            tracing::warn!(
                                "KVM_CREATE_PIT2 unavailable ({}), booting without PIT",
                                e
                            );
                            false
                        }
                    }
                }
                Err(e) => {
                    let split_available =
                        kvm.check_extension(sys::KVM_CAP_SPLIT_IRQCHIP).unwrap_or(0) > 0;
                    if split_available {
                        tracing::warn!(
                            "KVM full IRQCHIP failed ({e:#}); split IRQCHIP is available but Capsem does not yet emulate userspace IOAPIC/PIC"
                        );
                    }
                    return Err(e)
                        .context("KVM full IRQCHIP is required for x86_64 virtio-mmio interrupts");
                }
            }
        };

        // Pre-flight: on restricted/nested KVM, CPUID may be unsupported.
        // Same probe used in CI (.github/workflows/release.yaml).
        #[cfg(target_arch = "x86_64")]
        if let Err(e) = kvm.get_supported_cpuid() {
            tracing::warn!("KVM CPUID probe failed: {e:#}");
            tracing::warn!(
                "This indicates restricted/nested KVM -- vCPU creation will likely fail"
            );
        }

        // Create vCPUs (must happen before GIC init on aarch64)
        let mut vcpu_fds = Vec::new();
        for i in 0..config.cpu_count {
            match vm.create_vcpu(i) {
                Ok(vcpu) => vcpu_fds.push(vcpu),
                Err(e) => {
                    // On failure, run diagnostic probes to help debug restricted KVM
                    #[cfg(target_arch = "x86_64")]
                    run_kvm_diagnostics(&kvm);
                    return Err(e);
                }
            }
        }

        #[cfg(target_arch = "aarch64")]
        vm.create_gic(config.cpu_count)?;

        // -- Arch-specific: kernel loading --------------------------------
        #[cfg(target_arch = "aarch64")]
        let kernel_info = boot::load_kernel(&guest_mem, &config.kernel_path)?;

        #[cfg(target_arch = "x86_64")]
        let kernel_info = if restoring {
            None
        } else {
            Some(boot_x86_64::load_kernel(&guest_mem, &config.kernel_path)?)
        };

        // -- Arch-specific: initrd loading --------------------------------
        #[cfg(target_arch = "aarch64")]
        let initrd_info = config
            .initrd_path
            .as_ref()
            .map(|p| boot::load_initrd(&guest_mem, p, kernel_info.kernel_end))
            .transpose()?;

        #[cfg(target_arch = "x86_64")]
        let initrd_info = if let Some(kernel_info) = kernel_info.as_ref() {
            config
                .initrd_path
                .as_ref()
                .map(|p| boot_x86_64::load_initrd(&guest_mem, p, kernel_info.kernel_end))
                .transpose()?
        } else {
            None
        };

        #[cfg(target_arch = "x86_64")]
        let restored_checkpoint = if let Some(checkpoint_path) = config.checkpoint_path.as_deref() {
            Some(checkpoint::read_checkpoint(
                checkpoint_path,
                &guest_mem,
                config.cpu_count,
                virtio_mmio_device_count(config, vsock_ports),
            )?)
        } else {
            None
        };

        #[cfg(target_arch = "x86_64")]
        if let Some(restored) = restored_checkpoint.as_ref() {
            checkpoint::restore_vm(&vm, &restored.vm)?;
        }

        // -- Arch-specific: FDT (aarch64) / boot_params (x86_64) ---------
        #[cfg(target_arch = "aarch64")]
        {
            let mut virtio_devices = vec![fdt::VirtioDeviceInfo {
                base_addr: memory::virtio_mmio_addr(0),
                irq: memory::virtio_mmio_irq(0),
            }];
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
                cmdline: kernel_cmdline.clone(),
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
        if restored_checkpoint.is_some() {
            tracing::info!("KVM checkpoint restore: skipping cold boot x86_64 boot state setup");
        } else if let Some(kernel_info) = kernel_info.as_ref() {
            let cmdline = boot_x86_64::build_cmdline(
                &kernel_cmdline,
                virtio_mmio_device_count(config, vsock_ports),
                has_pit,
            );
            let e820 = memory::build_e820_map(config.ram_bytes);

            boot_x86_64::write_gdt(&guest_mem)?;
            boot_x86_64::write_page_tables(&guest_mem, memory::guest_phys_end(config.ram_bytes))?;
            boot_x86_64::write_acpi_tables(&guest_mem, config.cpu_count)?;
            boot_x86_64::write_boot_params(
                &guest_mem,
                &cmdline,
                initrd_info.as_ref(),
                &e820,
                &kernel_info.setup_header,
            )?;
            boot_x86_64::setup_cpuid(&kvm, &vcpu_fds[0], 0, config.cpu_count)?;
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
            // CPUID must be set on all vCPUs.
            let start = if restored_checkpoint.is_some() { 0 } else { 1 };
            for (vcpu_id, vcpu) in vcpu_fds.iter().enumerate().skip(start) {
                boot_x86_64::setup_cpuid(&kvm, vcpu, vcpu_id as u32, config.cpu_count)?;
                if restored_checkpoint.is_none() {
                    boot_x86_64::setup_application_processor(vcpu)?;
                }
            }
            if let Some(restored) = restored_checkpoint.as_ref() {
                checkpoint::restore_vcpus(&vcpu_fds, &restored.vcpus)?;
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
                anyhow::ensure!(
                    unsafe { libc::pipe(fds.as_mut_ptr()) } == 0,
                    "pipe() failed"
                );
                (fds[0], fds[1])
            };
            let (input_read, input_write) = {
                let mut fds = [0i32; 2];
                anyhow::ensure!(
                    unsafe { libc::pipe(fds.as_mut_ptr()) } == 0,
                    "pipe() failed"
                );
                (fds[0], fds[1])
            };
            (
                serial::KvmSerialConsole::new(output_read, input_write),
                output_write,
                input_read,
            )
        };

        serial_console.spawn_reader_with_log(config.serial_log_path.clone());

        let mmio_bus = Arc::new(mmio::MmioBus::new());
        #[cfg(target_arch = "x86_64")]
        let mut mmio_transports: Vec<(u32, Arc<virtio_mmio::VirtioMmioTransport>)> = Vec::new();
        let mut block_metrics: Vec<Arc<virtio_blk::BlockDeviceMetrics>> = Vec::new();
        #[cfg(target_arch = "x86_64")]
        let console_irq_fd = create_irq_eventfd()?;
        #[cfg(target_arch = "x86_64")]
        vm.irqfd(
            console_irq_fd.as_raw_fd(),
            irq_to_gsi(memory::virtio_mmio_irq(0)),
        )?;
        #[cfg(target_arch = "x86_64")]
        let console_mmio = virtio_mmio::VirtioMmioTransport::new_with_interrupt(
            Box::new(console_device),
            guest_mem.clone_ref(memory::RAM_BASE),
            console_irq_fd,
        );
        #[cfg(not(target_arch = "x86_64"))]
        let console_mmio = virtio_mmio::VirtioMmioTransport::new(
            Box::new(console_device),
            guest_mem.clone_ref(memory::RAM_BASE),
        );
        #[cfg(target_arch = "x86_64")]
        let console_mmio = {
            let transport = Arc::new(console_mmio);
            mmio_transports.push((0, Arc::clone(&transport)));
            transport
        };
        #[cfg(not(target_arch = "x86_64"))]
        let console_mmio = Arc::new(console_mmio);
        mmio_bus.register(
            memory::virtio_mmio_addr(0),
            memory::VIRTIO_MMIO_SIZE,
            console_mmio,
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
            #[cfg(target_arch = "x86_64")]
            let blk_irq_fd = create_irq_eventfd()?;
            #[cfg(target_arch = "x86_64")]
            let blk_interrupt_status = Arc::new(AtomicU32::new(0));
            #[cfg(target_arch = "x86_64")]
            vm.irqfd(
                blk_irq_fd.as_raw_fd(),
                irq_to_gsi(memory::virtio_mmio_irq(1)),
            )?;
            let blk_device = virtio_blk::VirtioBlockDevice::new(disk_path, true)?;
            block_metrics.push(blk_device.metrics());
            #[cfg(target_arch = "x86_64")]
            let blk_notify_fds = {
                let mut notify_fds = Vec::with_capacity(blk_device.queue_count());
                for queue_index in 0..blk_device.queue_count() {
                    let notify_fd = create_notify_eventfd()?;
                    vm.ioeventfd(
                        notify_fd.as_raw_fd(),
                        memory::virtio_mmio_addr(1) + virtio_mmio::QUEUE_NOTIFY_OFFSET,
                        4,
                        Some(queue_index as u64),
                    )?;
                    notify_fds.push(notify_fd);
                }
                notify_fds
            };
            #[cfg(target_arch = "x86_64")]
            let blk_device = blk_device.with_async_notify(
                blk_irq_fd.as_raw_fd(),
                Arc::clone(&blk_interrupt_status),
                blk_notify_fds,
            );
            #[cfg(target_arch = "x86_64")]
            let blk_mmio = virtio_mmio::VirtioMmioTransport::new_with_interrupt_status(
                Box::new(blk_device),
                guest_mem.clone_ref(memory::RAM_BASE),
                blk_irq_fd,
                blk_interrupt_status,
            );
            #[cfg(not(target_arch = "x86_64"))]
            let blk_mmio = virtio_mmio::VirtioMmioTransport::new(
                Box::new(blk_device),
                guest_mem.clone_ref(memory::RAM_BASE),
            );
            #[cfg(target_arch = "x86_64")]
            let blk_mmio = {
                let transport = Arc::new(blk_mmio);
                mmio_transports.push((1, Arc::clone(&transport)));
                transport
            };
            #[cfg(not(target_arch = "x86_64"))]
            let blk_mmio = Arc::new(blk_mmio);
            mmio_bus.register(
                memory::virtio_mmio_addr(1),
                memory::VIRTIO_MMIO_SIZE,
                blk_mmio,
            )?;
        }

        if let Some(ref scratch_path) = config.scratch_disk_path {
            #[cfg(target_arch = "x86_64")]
            let scratch_irq_fd = create_irq_eventfd()?;
            #[cfg(target_arch = "x86_64")]
            let scratch_interrupt_status = Arc::new(AtomicU32::new(0));
            #[cfg(target_arch = "x86_64")]
            vm.irqfd(
                scratch_irq_fd.as_raw_fd(),
                irq_to_gsi(memory::virtio_mmio_irq(2)),
            )?;
            let scratch_device = virtio_blk::VirtioBlockDevice::new(scratch_path, false)?;
            block_metrics.push(scratch_device.metrics());
            #[cfg(target_arch = "x86_64")]
            let scratch_notify_fds = {
                let mut notify_fds = Vec::with_capacity(scratch_device.queue_count());
                for queue_index in 0..scratch_device.queue_count() {
                    let notify_fd = create_notify_eventfd()?;
                    vm.ioeventfd(
                        notify_fd.as_raw_fd(),
                        memory::virtio_mmio_addr(2) + virtio_mmio::QUEUE_NOTIFY_OFFSET,
                        4,
                        Some(queue_index as u64),
                    )?;
                    notify_fds.push(notify_fd);
                }
                notify_fds
            };
            #[cfg(target_arch = "x86_64")]
            let scratch_device = scratch_device.with_async_notify(
                scratch_irq_fd.as_raw_fd(),
                Arc::clone(&scratch_interrupt_status),
                scratch_notify_fds,
            );
            #[cfg(target_arch = "x86_64")]
            let scratch_mmio = virtio_mmio::VirtioMmioTransport::new_with_interrupt_status(
                Box::new(scratch_device),
                guest_mem.clone_ref(memory::RAM_BASE),
                scratch_irq_fd,
                scratch_interrupt_status,
            );
            #[cfg(not(target_arch = "x86_64"))]
            let scratch_mmio = virtio_mmio::VirtioMmioTransport::new(
                Box::new(scratch_device),
                guest_mem.clone_ref(memory::RAM_BASE),
            );
            #[cfg(target_arch = "x86_64")]
            let scratch_mmio = {
                let transport = Arc::new(scratch_mmio);
                mmio_transports.push((2, Arc::clone(&transport)));
                transport
            };
            #[cfg(not(target_arch = "x86_64"))]
            let scratch_mmio = Arc::new(scratch_mmio);
            mmio_bus.register(
                memory::virtio_mmio_addr(2),
                memory::VIRTIO_MMIO_SIZE,
                scratch_mmio,
            )?;
        }

        // -- Shared: VirtioFS (slot 4+) -----------------------------------
        for (i, share) in config.virtio_fs_shares.iter().enumerate() {
            let slot = 4 + i as u32;
            let fs_irq_fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC) };
            anyhow::ensure!(fs_irq_fd >= 0, "failed to create eventfd for VirtioFS");
            let fs_irq_fd = unsafe { OwnedFd::from_raw_fd(fs_irq_fd) };
            let fs_interrupt_status = Arc::new(AtomicU32::new(0));

            let fs_gsi = irq_to_gsi(memory::virtio_mmio_irq(slot));
            vm.irqfd(fs_irq_fd.as_raw_fd(), fs_gsi)?;

            let fs_device = virtio_fs::VirtioFsDevice::new(
                &share.tag,
                &share.host_path,
                share.read_only,
                fs_irq_fd.as_raw_fd(),
                Arc::clone(&fs_interrupt_status),
            )?;
            let fs_mmio = virtio_mmio::VirtioMmioTransport::new_with_interrupt_status(
                Box::new(fs_device),
                guest_mem.clone_ref(memory::RAM_BASE),
                fs_irq_fd,
                fs_interrupt_status,
            );
            let fs_mmio = Arc::new(fs_mmio);
            #[cfg(target_arch = "x86_64")]
            mmio_transports.push((slot, Arc::clone(&fs_mmio)));
            mmio_bus.register(
                memory::virtio_mmio_addr(slot),
                memory::VIRTIO_MMIO_SIZE,
                fs_mmio,
            )?;
        }

        #[cfg(target_arch = "x86_64")]
        if let Some(pmem) = pmem_mapping.as_ref() {
            let slot = 4 + config.virtio_fs_shares.len() as u32;
            let pmem_irq_fd = create_irq_eventfd()?;
            let pmem_interrupt_status = Arc::new(AtomicU32::new(0));
            vm.irqfd(
                pmem_irq_fd.as_raw_fd(),
                irq_to_gsi(memory::virtio_mmio_irq(slot)),
            )?;
            let pmem_device = virtio_pmem::VirtioPmemDevice::new(pmem.guest_phys_addr, pmem.size);
            let pmem_mmio = virtio_mmio::VirtioMmioTransport::new_with_interrupt_status(
                Box::new(pmem_device),
                guest_mem.clone_ref(memory::RAM_BASE),
                pmem_irq_fd,
                pmem_interrupt_status,
            );
            let pmem_mmio = Arc::new(pmem_mmio);
            mmio_transports.push((slot, Arc::clone(&pmem_mmio)));
            mmio_bus.register(
                memory::virtio_mmio_addr(slot),
                memory::VIRTIO_MMIO_SIZE,
                pmem_mmio,
            )?;
        }

        // -- Shared: vsock ------------------------------------------------
        let (vsock_tx, vsock_rx) = mpsc::unbounded_channel();
        let shutdown = Arc::new(AtomicBool::new(false));
        let mut vsock_listener_handles = Vec::new();
        let mut vsock_irq_handles = Vec::new();

        if let Some(vsock_bindings) = vsock_bindings {
            let guest_cid = vsock_bindings.guest_cid();
            let vhost_fd = virtio_vsock::open_vhost_vsock()?;
            let (vsock_device, vsock_event_fds) =
                virtio_vsock::VhostVsockDevice::new(guest_cid, vhost_fd)?;
            let vsock_interrupt_status = Arc::new(AtomicU32::new(0));
            for (queue_index, &kick_fd) in vsock_event_fds.backend_kick_fds.iter().enumerate() {
                vm.ioeventfd(
                    kick_fd,
                    memory::virtio_mmio_addr(3) + virtio_mmio::QUEUE_NOTIFY_OFFSET,
                    4,
                    Some(queue_index as u64),
                )?;
            }

            let vsock_mmio = virtio_mmio::VirtioMmioTransport::new_with_shared_interrupt_status(
                Box::new(vsock_device),
                guest_mem.clone_ref(memory::RAM_BASE),
                Arc::clone(&vsock_interrupt_status),
            );
            let vsock_mmio = Arc::new(vsock_mmio);
            #[cfg(target_arch = "x86_64")]
            mmio_transports.push((3, Arc::clone(&vsock_mmio)));
            mmio_bus.register(
                memory::virtio_mmio_addr(3),
                memory::VIRTIO_MMIO_SIZE,
                vsock_mmio,
            )?;

            let vsock_gsi = irq_to_gsi(memory::virtio_mmio_irq(3));
            let mut irq_fds = Vec::with_capacity(vsock_event_fds.call_fds.len());
            for _ in &vsock_event_fds.call_fds {
                let irq_fd = create_irq_eventfd()?;
                vm.irqfd(irq_fd.as_raw_fd(), vsock_gsi)?;
                irq_fds.push(irq_fd);
            }
            vsock_irq_handles = virtio_vsock::spawn_call_irq_bridges(
                &vsock_event_fds.call_fds,
                irq_fds,
                vsock_interrupt_status,
                Arc::clone(&shutdown),
            )?;

            vsock_listener_handles = virtio_vsock::spawn_vsock_listeners(
                vsock_bindings,
                vsock_tx,
                Arc::clone(&shutdown),
            );
        }

        #[cfg(target_arch = "x86_64")]
        if let Some(restored) = restored_checkpoint.as_ref() {
            for snapshot in &restored.mmio_devices {
                let Some((_slot, transport)) = mmio_transports
                    .iter()
                    .find(|(slot, _transport)| *slot == snapshot.slot)
                else {
                    anyhow::bail!(
                        "checkpoint MMIO slot {} does not exist in restored VM",
                        snapshot.slot
                    );
                };
                transport.restore(&snapshot.transport)?;
            }
        }

        // -- Shared: spawn vCPU threads -----------------------------------
        let control = Arc::new(vcpu::VcpuControl::new(config.cpu_count));
        let mut vcpu_handles = Vec::new();
        for vcpu in vcpu_fds {
            let handle = vcpu::run_vcpu(
                vcpu,
                Arc::clone(&mmio_bus),
                #[cfg(target_arch = "x86_64")]
                Arc::clone(&pio_bus),
                Arc::clone(&control),
            );
            vcpu_handles.push(handle);
        }

        let handle = KvmHandle {
            state: std::sync::atomic::AtomicU8::new(VmState::Running as u8),
            serial: serial_console,
            shutdown,
            control,
            _vm: Some(vm),
            _vcpu_handles: vcpu_handles,
            _guest_mem: guest_mem,
            _mmio_bus: mmio_bus,
            block_metrics,
            #[cfg(target_arch = "x86_64")]
            _mmio_transports: mmio_transports,
            #[cfg(target_arch = "x86_64")]
            _pmem_mapping: pmem_mapping,
            _vsock_listener_handles: vsock_listener_handles,
            _vsock_irq_handles: vsock_irq_handles,
        };

        Ok((Box::new(handle), vsock_rx))
    }
}

/// A running KVM virtual machine.
struct KvmHandle {
    state: std::sync::atomic::AtomicU8,
    serial: serial::KvmSerialConsole,
    shutdown: Arc<AtomicBool>,
    control: Arc<vcpu::VcpuControl>,
    _vm: Option<sys::VmFd>,
    _vcpu_handles: Vec<std::thread::JoinHandle<Result<()>>>,
    _guest_mem: memory::GuestMemory,
    _mmio_bus: Arc<mmio::MmioBus>,
    block_metrics: Vec<Arc<virtio_blk::BlockDeviceMetrics>>,
    #[cfg(target_arch = "x86_64")]
    _mmio_transports: Vec<(u32, Arc<virtio_mmio::VirtioMmioTransport>)>,
    #[cfg(target_arch = "x86_64")]
    _pmem_mapping: Option<PmemMapping>,
    _vsock_listener_handles: Vec<std::thread::JoinHandle<()>>,
    _vsock_irq_handles: Vec<std::thread::JoinHandle<()>>,
}

// Safety: all fields are Send, vCPU threads are managed via JoinHandles.
unsafe impl Send for KvmHandle {}

impl VmHandle for KvmHandle {
    fn stop(&self) -> Result<()> {
        self.shutdown.store(true, Ordering::SeqCst);
        self.control.request_stop();
        self.state.store(VmState::Stopped as u8, Ordering::SeqCst);
        Ok(())
    }

    fn state(&self) -> VmState {
        state_from_u8(self.state.load(Ordering::SeqCst))
    }

    fn serial(&self) -> &dyn SerialConsole {
        &self.serial
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn pause(&self) -> Result<()> {
        if self.state() == VmState::Stopped {
            anyhow::bail!("cannot pause stopped KVM VM");
        }
        self.state.store(VmState::Pausing as u8, Ordering::SeqCst);
        match self.control.request_pause(KVM_PAUSE_TIMEOUT) {
            Ok(()) => {
                self.state.store(VmState::Paused as u8, Ordering::SeqCst);
                Ok(())
            }
            Err(e) => {
                self.state.store(VmState::Running as u8, Ordering::SeqCst);
                Err(e)
            }
        }
    }

    fn resume(&self) -> Result<()> {
        if self.state() == VmState::Stopped {
            anyhow::bail!("cannot resume stopped KVM VM");
        }
        self.state.store(VmState::Resuming as u8, Ordering::SeqCst);
        match self.control.resume() {
            Ok(()) => {
                self.state.store(VmState::Running as u8, Ordering::SeqCst);
                Ok(())
            }
            Err(e) => {
                self.state.store(VmState::Paused as u8, Ordering::SeqCst);
                Err(e)
            }
        }
    }

    fn save_state(&self, path: &std::path::Path) -> Result<()> {
        match self.state() {
            VmState::Paused => {}
            VmState::Stopped => anyhow::bail!("cannot save stopped KVM VM"),
            state => {
                anyhow::bail!("KVM VM must be paused before save_state, current state={state}")
            }
        }
        self.state.store(VmState::Saving as u8, Ordering::SeqCst);
        #[cfg(target_arch = "x86_64")]
        let result = self.control.snapshots().and_then(|snapshots| {
            for (_slot, transport) in &self._mmio_transports {
                transport.quiesce()?;
            }
            #[cfg(test)]
            let vm_snapshot = if let Some(vm) = self._vm.as_ref() {
                checkpoint::snapshot_vm(vm)?
            } else {
                checkpoint::VmSnapshot::default()
            };
            #[cfg(not(test))]
            let vm_snapshot = self
                ._vm
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("missing KVM VM fd for checkpoint save"))
                .and_then(checkpoint::snapshot_vm)?;
            let mmio_snapshots: Vec<_> = self
                ._mmio_transports
                .iter()
                .map(|(slot, transport)| checkpoint::MmioDeviceSnapshot {
                    slot: *slot,
                    transport: transport.snapshot(),
                })
                .collect();
            checkpoint::write_checkpoint(
                path,
                &self._guest_mem,
                &snapshots,
                &vm_snapshot,
                &mmio_snapshots,
            )
        });
        #[cfg(not(target_arch = "x86_64"))]
        let result = Err(anyhow::anyhow!(
            "KVM save_state is only implemented for x86_64"
        ));
        self.state.store(VmState::Paused as u8, Ordering::SeqCst);
        result
    }

    fn supports_checkpoint(&self) -> bool {
        cfg!(target_arch = "x86_64")
    }

    fn hypervisor_metrics(&self) -> VmHypervisorMetrics {
        let mut block = VmBlockMetrics::default();
        for metrics in &self.block_metrics {
            let snapshot = metrics.snapshot();
            block.queue_notifications_total += snapshot.queue_notifications_total;
            block.queue_drains_total += snapshot.queue_drains_total;
            block.descriptors_drained_total += snapshot.descriptors_drained_total;
            block.used_entries_total += snapshot.used_entries_total;
            block.interrupts_raised_total += snapshot.interrupts_raised_total;
            block.interrupts_suppressed_total += snapshot.interrupts_suppressed_total;
            block.read_ops_total += snapshot.read_ops_total;
            block.write_ops_total += snapshot.write_ops_total;
            block.bytes_read_total += snapshot.bytes_read_total;
            block.bytes_written_total += snapshot.bytes_written_total;
            block.requests_total += snapshot.requests_total;
            block.request_bytes_total += snapshot.request_bytes_total;
            block.request_duration_micros_total += snapshot.request_duration_micros_total;
            block.queue_drain_duration_micros_total += snapshot.queue_drain_duration_micros_total;
            block.max_request_bytes = block.max_request_bytes.max(snapshot.max_request_bytes);
            block.max_data_descriptors_per_request = block
                .max_data_descriptors_per_request
                .max(snapshot.max_data_descriptors_per_request);
            block.max_requests_per_drain = block
                .max_requests_per_drain
                .max(snapshot.max_requests_per_drain);
            block.async_submissions_total += snapshot.async_submissions_total;
            block.async_completions_total += snapshot.async_completions_total;
            block.async_fallbacks_total += snapshot.async_fallbacks_total;
            block.async_queue_full_total += snapshot.async_queue_full_total;
            block.async_in_flight += snapshot.async_in_flight;
        }
        VmHypervisorMetrics { block }
    }
}

fn state_from_u8(val: u8) -> VmState {
    match val {
        x if x == VmState::Running as u8 => VmState::Running,
        x if x == VmState::Paused as u8 => VmState::Paused,
        x if x == VmState::Pausing as u8 => VmState::Pausing,
        x if x == VmState::Resuming as u8 => VmState::Resuming,
        x if x == VmState::Saving as u8 => VmState::Saving,
        x if x == VmState::Stopped as u8 => VmState::Stopped,
        _ => VmState::Unknown,
    }
}

/// Run diagnostic probes when vCPU creation fails.
/// Logs results at ERROR level so they appear in the output without RUST_LOG=debug.
#[cfg(target_arch = "x86_64")]
fn run_kvm_diagnostics(kvm: &sys::KvmFd) {
    tracing::error!("--- KVM diagnostic probes (vCPU creation failed) ---");

    // Probe 1: kernel info
    if let Ok(uname) = nix_uname() {
        tracing::error!("kernel: {} ({})", uname.release, uname.machine);
    }

    // Probe 2: check nested KVM
    for path in &[
        "/sys/module/kvm_intel/parameters/nested",
        "/sys/module/kvm_amd/parameters/nested",
    ] {
        if let Ok(val) = std::fs::read_to_string(path) {
            tracing::error!("nested KVM ({path}): {}", val.trim());
        }
    }

    // Probe 3: capabilities
    if let Ok(nr) = kvm.check_extension(sys::KVM_CAP_NR_VCPUS) {
        tracing::error!("KVM_CAP_NR_VCPUS = {nr}");
    }
    if let Ok(max) = kvm.check_extension(sys::KVM_CAP_MAX_VCPUS) {
        tracing::error!("KVM_CAP_MAX_VCPUS = {max}");
    }

    // Probe 4: create a fresh VM and try vcpu WITHOUT irqchip
    tracing::error!("probe: creating fresh VM without IRQCHIP...");
    match kvm.create_vm() {
        Ok(probe_vm) => match probe_vm.create_vcpu(0) {
            Ok(_vcpu) => {
                tracing::error!(
                    "probe: vCPU(0) succeeds WITHOUT IRQCHIP -- IRQCHIP causes the conflict"
                );
            }
            Err(e) => {
                tracing::error!("probe: vCPU(0) fails even WITHOUT IRQCHIP: {e:#}");
                tracing::error!("probe: this KVM environment cannot create vCPUs at all");
            }
        },
        Err(e) => {
            tracing::error!("probe: fresh VM creation failed: {e:#}");
        }
    }

    tracing::error!("--- end KVM diagnostics ---");
    tracing::error!("For detailed probing, run: python3 scripts/kvm-diagnostic.py");
}

/// Minimal uname wrapper for diagnostics.
#[cfg(target_arch = "x86_64")]
fn nix_uname() -> std::io::Result<UnameInfo> {
    let mut buf: libc::utsname = unsafe { std::mem::zeroed() };
    if unsafe { libc::uname(&mut buf) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let release = unsafe { std::ffi::CStr::from_ptr(buf.release.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    let machine = unsafe { std::ffi::CStr::from_ptr(buf.machine.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    Ok(UnameInfo { release, machine })
}

#[cfg(target_arch = "x86_64")]
struct UnameInfo {
    release: String,
    machine: String,
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

    fn test_handle() -> KvmHandle {
        test_handle_with_control(Arc::new(vcpu::VcpuControl::new(0)))
    }

    fn test_handle_with_control(control: Arc<vcpu::VcpuControl>) -> KvmHandle {
        KvmHandle {
            state: std::sync::atomic::AtomicU8::new(VmState::Running as u8),
            serial: serial::KvmSerialConsole::new(-1, -1),
            shutdown: Arc::new(AtomicBool::new(false)),
            control,
            _vm: None,
            _vcpu_handles: Vec::new(),
            _guest_mem: memory::GuestMemory::new(4096).unwrap(),
            _mmio_bus: Arc::new(mmio::MmioBus::new()),
            block_metrics: Vec::new(),
            #[cfg(target_arch = "x86_64")]
            _mmio_transports: Vec::new(),
            #[cfg(target_arch = "x86_64")]
            _pmem_mapping: None,
            _vsock_listener_handles: Vec::new(),
            _vsock_irq_handles: Vec::new(),
        }
    }

    #[cfg(target_arch = "x86_64")]
    fn snapshot(id: u32) -> checkpoint::VcpuSnapshot {
        let regs = sys::KvmRegs {
            rip: 0x1000 + id as u64,
            ..Default::default()
        };
        checkpoint::VcpuSnapshot {
            id,
            regs,
            sregs: sys::KvmSregs::default(),
            mp_state: sys::KvmMpState {
                mp_state: sys::KVM_MP_STATE_RUNNABLE,
            },
            msrs: Vec::new(),
            lapic: sys::KvmLapicState::default(),
            events: sys::KvmVcpuEvents::default(),
            debugregs: sys::KvmDebugRegs::default(),
            fpu: sys::KvmFpu::default(),
            xcrs: sys::KvmXcrs::default(),
            xsave: sys::KvmXsave::default(),
        }
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("capsem-kvm-handle").join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
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
    fn kvm_handle_supports_checkpoint_trait() {
        let handle = test_handle();
        assert_eq!(handle.supports_checkpoint(), cfg!(target_arch = "x86_64"));
    }

    #[test]
    fn kvm_pause_resume_update_state() {
        let handle = test_handle();

        handle.pause().unwrap();
        assert_eq!(handle.state(), VmState::Paused);

        handle.resume().unwrap();
        assert_eq!(handle.state(), VmState::Running);
    }

    #[test]
    fn kvm_save_state_requires_pause() {
        let handle = test_handle();
        let path = temp_dir("save-requires-pause").join("state.kvm");

        let err = handle.save_state(&path).unwrap_err();

        assert!(err
            .to_string()
            .contains("KVM VM must be paused before save_state"));
        assert!(!path.exists());
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn kvm_save_state_writes_checkpoint_file() {
        let control = Arc::new(vcpu::VcpuControl::new(1));
        let waiter = {
            let control = Arc::clone(&control);
            std::thread::spawn(move || loop {
                control.wait_if_paused(0, || Ok(snapshot(0))).unwrap();
                if control.is_stopped() {
                    break;
                }
                std::thread::yield_now();
            })
        };
        let handle = test_handle_with_control(control);
        let path = temp_dir("save-writes").join("state.kvm");

        handle.pause().unwrap();
        handle.save_state(&path).unwrap();

        assert_eq!(handle.state(), VmState::Paused);
        let meta = std::fs::metadata(path).unwrap();
        assert_eq!(meta.len(), 44 + 4 + 6952 + 1720 + 4096);
        handle.resume().unwrap();
        handle.stop().unwrap();
        waiter.join().unwrap();
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn kvm_save_state_restores_paused_state_after_error() {
        let handle = test_handle();
        let path = temp_dir("save-error").join("missing").join("state.kvm");

        handle.pause().unwrap();
        let err = handle.save_state(&path).unwrap_err();

        assert!(err
            .to_string()
            .contains("checkpoint parent directory does not exist"));
        assert_eq!(handle.state(), VmState::Paused);
    }

    #[test]
    fn kvm_stop_blocks_lifecycle_ops() {
        let handle = test_handle();

        handle.stop().unwrap();

        assert_eq!(handle.state(), VmState::Stopped);
        assert!(handle.pause().unwrap_err().to_string().contains("stopped"));
        assert!(handle.resume().unwrap_err().to_string().contains("stopped"));
        assert!(handle
            .save_state(&temp_dir("stopped").join("state.kvm"))
            .unwrap_err()
            .to_string()
            .contains("stopped"));
    }

    #[test]
    fn kvm_state_decoder_preserves_transient_states() {
        assert_eq!(state_from_u8(VmState::Pausing as u8), VmState::Pausing);
        assert_eq!(state_from_u8(VmState::Resuming as u8), VmState::Resuming);
        assert_eq!(state_from_u8(VmState::Saving as u8), VmState::Saving);
        assert_eq!(state_from_u8(255), VmState::Unknown);
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn pmem_rootfs_gate_requires_disk_and_opt_in() {
        let mut config = VmConfig {
            cpu_count: 1,
            ram_bytes: 4096,
            kernel_path: "/nonexistent/vmlinuz".into(),
            initrd_path: None,
            disk_path: None,
            scratch_disk_path: None,
            virtio_fs_shares: Vec::new(),
            kernel_cmdline: String::new(),
            expected_kernel_hash: None,
            expected_initrd_hash: None,
            checkpoint_path: None,
            expected_disk_hash: None,
            machine_identifier_path: None,
            serial_log_path: None,
        };

        std::env::remove_var("CAPSEM_KVM_ROOTFS_PMEM_DAX");
        std::env::remove_var("CAPSEM_KVM_ROOTFS_PMEM_FILE_BACKED");
        std::env::remove_var("CAPSEM_KVM_ROOTFS_PMEM_POPULATE");
        assert!(!should_attach_pmem_rootfs(&config));
        assert!(!should_use_file_backed_pmem_rootfs());
        assert!(!env_truthy("CAPSEM_KVM_ROOTFS_PMEM_POPULATE"));
        assert_eq!(virtio_mmio_device_count(&config, &[]), 1);

        std::env::set_var("CAPSEM_KVM_ROOTFS_PMEM_DAX", "1");
        std::env::set_var("CAPSEM_KVM_ROOTFS_PMEM_FILE_BACKED", "yes");
        std::env::set_var("CAPSEM_KVM_ROOTFS_PMEM_POPULATE", "on");
        assert!(!should_attach_pmem_rootfs(&config));
        assert!(should_use_file_backed_pmem_rootfs());
        assert!(env_truthy("CAPSEM_KVM_ROOTFS_PMEM_POPULATE"));

        config.disk_path = Some("/tmp/rootfs.erofs".into());
        assert!(should_attach_pmem_rootfs(&config));
        assert_eq!(virtio_mmio_device_count(&config, &[]), 3);

        std::env::remove_var("CAPSEM_KVM_ROOTFS_PMEM_DAX");
        std::env::remove_var("CAPSEM_KVM_ROOTFS_PMEM_FILE_BACKED");
        std::env::remove_var("CAPSEM_KVM_ROOTFS_PMEM_POPULATE");
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn pmem_region_is_section_aligned_for_zone_device() {
        assert_eq!(pmem_guest_phys_addr(8 * 1024 * 1024 * 1024), 0x2400_00000);

        let image_size = 805_425_152;
        let size = pmem_region_size(image_size);
        assert!(size >= image_size);
        assert_eq!(size % KVM_PMEM_ALIGNMENT, 0);
        assert_eq!(size, 896 * 1024 * 1024);
        assert!(!is_pmem_file_backing_size(image_size));
        assert!(is_pmem_file_backing_size(size));
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn pmem_madvise_policy_parses_env() {
        std::env::remove_var("CAPSEM_KVM_ROOTFS_PMEM_MADVISE");
        assert_eq!(PmemMadvise::from_env().unwrap(), PmemMadvise::None);

        std::env::set_var("CAPSEM_KVM_ROOTFS_PMEM_MADVISE", "sequential");
        assert_eq!(PmemMadvise::from_env().unwrap(), PmemMadvise::Sequential);

        std::env::set_var("CAPSEM_KVM_ROOTFS_PMEM_MADVISE", "will-need");
        assert_eq!(PmemMadvise::from_env().unwrap(), PmemMadvise::WillNeed);

        std::env::set_var("CAPSEM_KVM_ROOTFS_PMEM_MADVISE", "bad");
        assert!(PmemMadvise::from_env().is_err());

        std::env::remove_var("CAPSEM_KVM_ROOTFS_PMEM_MADVISE");
    }

    #[cfg(not(target_arch = "x86_64"))]
    #[test]
    fn kvm_boot_rejects_checkpoint_path_on_unsupported_arch() {
        let h = KvmHypervisor;
        let config = VmConfig {
            cpu_count: 1,
            ram_bytes: 4096,
            kernel_path: "/nonexistent/vmlinuz".into(),
            initrd_path: None,
            disk_path: None,
            scratch_disk_path: None,
            virtio_fs_shares: Vec::new(),
            kernel_cmdline: String::new(),
            expected_kernel_hash: None,
            expected_initrd_hash: None,
            checkpoint_path: Some("/tmp/checkpoint.kvm".into()),
            expected_disk_hash: None,
            machine_identifier_path: None,
            serial_log_path: None,
        };

        let err = match h.boot(&config, &[]) {
            Ok(_) => panic!("boot should reject checkpoint_path"),
            Err(err) => err,
        };

        assert!(err
            .to_string()
            .contains("KVM checkpoint restore is only implemented for x86_64"));
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
