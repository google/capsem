//! Safe Rust wrappers for KVM ioctls.
//!
//! Wraps libc::ioctl directly -- no kvm-ioctls or kvm-bindings crate.
//! Each wrapper type owns its fd and unmaps/closes on drop.

use std::os::unix::io::{AsRawFd, FromRawFd, OwnedFd, RawFd};

use anyhow::{Context, Result, bail};

// ---------------------------------------------------------------------------
// ioctl encoding helpers (Linux ioctl number scheme)
// ---------------------------------------------------------------------------

const fn _io(ty: u32, nr: u32) -> u64 {
    ((ty as u64) << 8) | (nr as u64)
}

const fn _iow(ty: u32, nr: u32, size: u32) -> u64 {
    (1u64 << 30) | ((size as u64) << 16) | ((ty as u64) << 8) | (nr as u64)
}

const fn _ior(ty: u32, nr: u32, size: u32) -> u64 {
    (2u64 << 30) | ((size as u64) << 16) | ((ty as u64) << 8) | (nr as u64)
}

const fn _iowr(ty: u32, nr: u32, size: u32) -> u64 {
    (3u64 << 30) | ((size as u64) << 16) | ((ty as u64) << 8) | (nr as u64)
}

// ---------------------------------------------------------------------------
// KVM ioctl numbers
// ---------------------------------------------------------------------------

const KVMIO: u32 = 0xAE;

// System ioctls (on /dev/kvm fd)
pub(super) const KVM_GET_API_VERSION: u64 = _io(KVMIO, 0x00);
pub(super) const KVM_CREATE_VM: u64 = _io(KVMIO, 0x01);
pub(super) const KVM_CHECK_EXTENSION: u64 = _io(KVMIO, 0x03);
pub(super) const KVM_GET_VCPU_MMAP_SIZE: u64 = _io(KVMIO, 0x04);

// VM ioctls (on VM fd)
pub(super) const KVM_SET_USER_MEMORY_REGION: u64 = _iow(KVMIO, 0x46, 32); // sizeof kvm_userspace_memory_region
pub(super) const KVM_CREATE_VCPU: u64 = _io(KVMIO, 0x41);
pub(super) const KVM_CREATE_DEVICE: u64 = _iowr(KVMIO, 0xE0, 12); // sizeof kvm_create_device
pub(super) const KVM_IRQFD: u64 = _iow(KVMIO, 0x76, 32); // sizeof kvm_irqfd
pub(super) const KVM_IOEVENTFD: u64 = _iow(KVMIO, 0x79, 64); // sizeof kvm_ioeventfd

// vCPU ioctls (on vCPU fd)
pub(super) const KVM_RUN: u64 = _io(KVMIO, 0x80);

// ---------------------------------------------------------------------------
// ARM64-specific ioctl numbers
// ---------------------------------------------------------------------------

#[cfg(target_arch = "aarch64")]
pub(super) const KVM_GET_ONE_REG: u64 = _iow(KVMIO, 0xAB, 16); // sizeof kvm_one_reg
#[cfg(target_arch = "aarch64")]
pub(super) const KVM_SET_ONE_REG: u64 = _iow(KVMIO, 0xAC, 16);
#[cfg(target_arch = "aarch64")]
pub(super) const KVM_ARM_VCPU_INIT: u64 = _iow(KVMIO, 0xAE, 36); // sizeof kvm_vcpu_init
#[cfg(target_arch = "aarch64")]
pub(super) const KVM_ARM_PREFERRED_TARGET: u64 = _ior(KVMIO, 0xAF, 36);
#[cfg(target_arch = "aarch64")]
pub(super) const KVM_SET_DEVICE_ATTR: u64 = _iow(KVMIO, 0xE1, 24); // sizeof kvm_device_attr

// ---------------------------------------------------------------------------
// KVM capability IDs
// ---------------------------------------------------------------------------

pub(super) const KVM_CAP_IRQFD: u32 = 32;
pub(super) const KVM_CAP_NR_VCPUS: u32 = 9;
pub(super) const KVM_CAP_MAX_VCPUS: u32 = 66;

#[cfg(target_arch = "aarch64")]
pub(super) const KVM_CAP_ONE_REG: u32 = 70;
#[cfg(target_arch = "aarch64")]
pub(super) const KVM_CAP_ARM_VM_IPA_SIZE: u32 = 165;

// ---------------------------------------------------------------------------
// KVM exit reasons
// ---------------------------------------------------------------------------

pub(super) const KVM_EXIT_UNKNOWN: u32 = 0;
pub(super) const KVM_EXIT_MMIO: u32 = 6;
pub(super) const KVM_EXIT_SYSTEM_EVENT: u32 = 24;
pub(super) const KVM_EXIT_INTERNAL_ERROR: u32 = 17;

// System event types
pub(super) const KVM_SYSTEM_EVENT_SHUTDOWN: u32 = 1;
pub(super) const KVM_SYSTEM_EVENT_RESET: u32 = 2;

// ---------------------------------------------------------------------------
// GIC constants (ARM64 only)
// ---------------------------------------------------------------------------

#[cfg(target_arch = "aarch64")]
pub(super) const KVM_DEV_TYPE_ARM_VGIC_V3: u32 = 5;
#[cfg(target_arch = "aarch64")]
pub(super) const KVM_DEV_ARM_VGIC_GRP_ADDR: u32 = 0;
#[cfg(target_arch = "aarch64")]
pub(super) const KVM_DEV_ARM_VGIC_GRP_NR_IRQS: u32 = 3;
#[cfg(target_arch = "aarch64")]
pub(super) const KVM_DEV_ARM_VGIC_GRP_CTRL: u32 = 4;
#[cfg(target_arch = "aarch64")]
pub(super) const KVM_VGIC_V3_ADDR_TYPE_DIST: u64 = 0;
#[cfg(target_arch = "aarch64")]
pub(super) const KVM_VGIC_V3_ADDR_TYPE_REDIST: u64 = 1;
#[cfg(target_arch = "aarch64")]
pub(super) const KVM_DEV_ARM_VGIC_CTRL_INIT: u64 = 0;

// ---------------------------------------------------------------------------
// Vhost ioctl numbers (/dev/vhost-vsock)
// ---------------------------------------------------------------------------

const VHOST: u32 = 0xAF;

pub(super) const VHOST_SET_OWNER: u64 = _io(VHOST, 0x01);
pub(super) const VHOST_SET_MEM_TABLE: u64 = _iow(VHOST, 0x03, 8); // sizeof(vhost_memory) base (flexible array)
pub(super) const VHOST_SET_VRING_NUM: u64 = _iow(VHOST, 0x10, 8); // sizeof(vhost_vring_state)
pub(super) const VHOST_SET_VRING_ADDR: u64 = _iow(VHOST, 0x11, 48); // sizeof(vhost_vring_addr)
pub(super) const VHOST_SET_VRING_BASE: u64 = _iow(VHOST, 0x12, 8); // sizeof(vhost_vring_state)
pub(super) const VHOST_SET_VRING_KICK: u64 = _iow(VHOST, 0x20, 8); // sizeof(vhost_vring_file)
pub(super) const VHOST_SET_VRING_CALL: u64 = _iow(VHOST, 0x21, 8); // sizeof(vhost_vring_file)
pub(super) const VHOST_VSOCK_SET_GUEST_CID: u64 = _iow(VHOST, 0x60, 8); // sizeof(u64)

// ---------------------------------------------------------------------------
// Vhost repr(C) structs
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(super) struct VhostVringState {
    pub index: u32,
    pub num: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(super) struct VhostVringAddr {
    pub index: u32,
    pub flags: u32,
    pub desc_user_addr: u64,
    pub used_user_addr: u64,
    pub avail_user_addr: u64,
    pub log_guest_addr: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(super) struct VhostVringFile {
    pub index: u32,
    pub fd: i32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(super) struct VhostMemoryRegion {
    pub guest_phys_addr: u64,
    pub memory_size: u64,
    pub userspace_addr: u64,
    pub flags_padding: u64,
}

// ---------------------------------------------------------------------------
// ARM64 register IDs (aarch64 only)
// ---------------------------------------------------------------------------

#[cfg(target_arch = "aarch64")]
pub(super) const KVM_REG_ARM64: u64 = 0x6000_0000_0000_0000;
#[cfg(target_arch = "aarch64")]
pub(super) const KVM_REG_SIZE_U64: u64 = 0x0030_0000_0000_0000;
#[cfg(target_arch = "aarch64")]
pub(super) const KVM_REG_ARM_CORE: u64 = 0x0000_0000_0010_0000;

/// Build an ARM64 core register ID from a u32 offset into kvm_regs.
#[cfg(target_arch = "aarch64")]
pub(super) const fn arm64_core_reg(offset: u64) -> u64 {
    KVM_REG_ARM64 | KVM_REG_SIZE_U64 | KVM_REG_ARM_CORE | offset
}

// X registers: each u64 is 2 u32 offsets apart
#[cfg(target_arch = "aarch64")]
pub(super) const REG_X0: u64 = arm64_core_reg(0x00);
#[cfg(target_arch = "aarch64")]
pub(super) const REG_X1: u64 = arm64_core_reg(0x02);
#[cfg(target_arch = "aarch64")]
pub(super) const REG_X2: u64 = arm64_core_reg(0x04);
#[cfg(target_arch = "aarch64")]
pub(super) const REG_X3: u64 = arm64_core_reg(0x06);
#[cfg(target_arch = "aarch64")]
pub(super) const REG_PC: u64 = arm64_core_reg(0x40);
#[cfg(target_arch = "aarch64")]
pub(super) const REG_PSTATE: u64 = arm64_core_reg(0x42);

// PSTATE value for EL1h with DAIF masked
#[cfg(target_arch = "aarch64")]
pub(super) const PSTATE_EL1H_DAIF: u64 = 0x3C5;

// ---------------------------------------------------------------------------
// repr(C) structs matching kernel headers
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(super) struct KvmUserspaceMemoryRegion {
    pub slot: u32,
    pub flags: u32,
    pub guest_phys_addr: u64,
    pub memory_size: u64,
    pub userspace_addr: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(super) struct KvmCreateDevice {
    pub type_: u32,
    pub fd: u32,
    pub flags: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(super) struct KvmDeviceAttr {
    pub flags: u32,
    pub group: u32,
    pub attr: u64,
    pub addr: u64,
}

#[cfg(target_arch = "aarch64")]
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(super) struct KvmOneReg {
    pub id: u64,
    pub addr: u64,
}

#[cfg(target_arch = "aarch64")]
#[repr(C)]
#[derive(Debug, Clone)]
pub(super) struct KvmVcpuInit {
    pub target: u32,
    pub features: [u32; 7],
}

#[cfg(target_arch = "aarch64")]
pub(super) const KVM_ARM_VCPU_POWER_OFF: u32 = 0;
#[cfg(target_arch = "aarch64")]
pub(super) const KVM_ARM_VCPU_PSCI_0_2: u32 = 2;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(super) struct KvmIrqfd {
    pub fd: u32,
    pub gsi: u32,
    pub flags: u32,
    pub resamplefd: u32,
    pub pad: [u8; 16],
}

/// kvm_run MMIO exit data (at offset 32 in the kvm_run mmap'd region).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(super) struct KvmRunMmio {
    pub phys_addr: u64,
    pub data: [u8; 8],
    pub len: u32,
    pub is_write: u8,
}

/// kvm_run system_event exit data.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(super) struct KvmRunSystemEvent {
    pub type_: u32,
    pub ndata: u32,
    pub data: [u64; 16],
}

// Offset of exit_reason within kvm_run struct
pub(super) const KVM_RUN_EXIT_REASON_OFFSET: usize = 8;
// Offset of the exit union data within kvm_run struct
pub(super) const KVM_RUN_EXIT_DATA_OFFSET: usize = 32;

// ---------------------------------------------------------------------------
// Compile-time struct size assertions (wrong size = kernel ABI violation)
// ---------------------------------------------------------------------------

const _: () = {
    assert!(std::mem::size_of::<KvmUserspaceMemoryRegion>() == 32);
    assert!(std::mem::size_of::<KvmCreateDevice>() == 12);
    assert!(std::mem::size_of::<KvmDeviceAttr>() == 24);
    assert!(std::mem::size_of::<KvmIrqfd>() == 32);
};

#[cfg(target_arch = "aarch64")]
const _: () = {
    assert!(std::mem::size_of::<KvmOneReg>() == 16);
    assert!(std::mem::size_of::<KvmVcpuInit>() == 32);
};

// ---------------------------------------------------------------------------
// KvmFd: /dev/kvm file descriptor
// ---------------------------------------------------------------------------

/// Owned handle to `/dev/kvm`.
pub(super) struct KvmFd {
    fd: OwnedFd,
}

impl KvmFd {
    /// Open `/dev/kvm` and verify API version.
    pub fn open() -> Result<Self> {
        if !std::path::Path::new("/dev/kvm").exists() {
            bail!(
                "/dev/kvm not found. KVM is required for VM boot on Linux. \
                 Check: (1) CPU supports virtualization (VT-x/AMD-V), \
                 (2) it is enabled in BIOS/UEFI, \
                 (3) kvm module is loaded (`sudo modprobe kvm_intel` or `kvm_amd`)"
            );
        }
        let raw = unsafe {
            libc::open(b"/dev/kvm\0".as_ptr() as *const libc::c_char, libc::O_RDWR | libc::O_CLOEXEC)
        };
        if raw < 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EACCES) {
                bail!(
                    "/dev/kvm: permission denied. Add your user to the 'kvm' group: \
                     sudo usermod -aG kvm $USER (then log out and back in)"
                );
            }
            bail!("/dev/kvm: {err}");
        }
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        let kvm = Self { fd };

        let version = kvm.ioctl(KVM_GET_API_VERSION, 0)?;
        if version != 12 {
            bail!("KVM API version {version}, expected 12");
        }

        // Log KVM capabilities for diagnostics
        tracing::info!("KVM API version {version}");
        if let Ok(nr) = kvm.check_extension(KVM_CAP_NR_VCPUS) {
            tracing::debug!("KVM_CAP_NR_VCPUS = {nr}");
        }
        if let Ok(max) = kvm.check_extension(KVM_CAP_MAX_VCPUS) {
            tracing::debug!("KVM_CAP_MAX_VCPUS = {max}");
        }

        Ok(kvm)
    }

    /// Check if a KVM extension/capability is supported.
    pub fn check_extension(&self, cap: u32) -> Result<i32> {
        self.ioctl(KVM_CHECK_EXTENSION, cap as u64)
    }

    /// Get the size of the mmap region for vCPU fds.
    pub fn vcpu_mmap_size(&self) -> Result<usize> {
        let size = self.ioctl(KVM_GET_VCPU_MMAP_SIZE, 0)?;
        Ok(size as usize)
    }

    /// Create a new VM, returning its fd wrapper.
    pub fn create_vm(&self) -> Result<VmFd> {
        let raw = self.ioctl(KVM_CREATE_VM, 0)?;
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        let mmap_size = self.vcpu_mmap_size()?;
        Ok(VmFd { fd, vcpu_mmap_size: mmap_size })
    }

    fn ioctl(&self, request: u64, arg: u64) -> Result<i32> {
        let ret = unsafe { libc::ioctl(self.fd.as_raw_fd(), request as libc::c_ulong, arg) };
        if ret < 0 {
            bail!(
                "KVM ioctl 0x{:x} failed: {}",
                request,
                std::io::Error::last_os_error()
            );
        }
        Ok(ret)
    }
}

// ---------------------------------------------------------------------------
// VmFd: VM file descriptor
// ---------------------------------------------------------------------------

/// Owned handle to a KVM VM.
pub(super) struct VmFd {
    fd: OwnedFd,
    vcpu_mmap_size: usize,
}

impl VmFd {
    /// Raw file descriptor for the VM (needed by vhost ioctls).
    pub fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }

    /// Register a guest memory region with KVM.
    pub fn set_user_memory_region(
        &self,
        slot: u32,
        guest_phys_addr: u64,
        memory_size: u64,
        userspace_addr: *const u8,
    ) -> Result<()> {
        let region = KvmUserspaceMemoryRegion {
            slot,
            flags: 0,
            guest_phys_addr,
            memory_size,
            userspace_addr: userspace_addr as u64,
        };
        let ret = unsafe {
            libc::ioctl(
                self.fd.as_raw_fd(),
                KVM_SET_USER_MEMORY_REGION as libc::c_ulong,
                &region as *const _ as u64,
            )
        };
        if ret < 0 {
            bail!(
                "KVM_SET_USER_MEMORY_REGION failed: {}",
                std::io::Error::last_os_error()
            );
        }
        Ok(())
    }

    /// Create a vCPU, returning its fd wrapper with mmap'd kvm_run region.
    pub fn create_vcpu(&self, id: u32) -> Result<VcpuFd> {
        let raw = unsafe {
            libc::ioctl(
                self.fd.as_raw_fd(),
                KVM_CREATE_VCPU as libc::c_ulong,
                id as u64,
            )
        };
        if raw < 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EEXIST) {
                bail!(
                    "KVM_CREATE_VCPU({id}) failed: vCPU already exists (EEXIST). \
                     This typically indicates a restricted or nested KVM environment \
                     (e.g., cloud workstation, CI runner) where the hypervisor \
                     pre-creates vCPU state. Capsem requires unrestricted KVM access. \
                     Debug: run `python3 scripts/kvm-diagnostic.py` for detailed probing."
                );
            }
            bail!("KVM_CREATE_VCPU({id}) failed: {err}");
        }
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };

        // mmap the kvm_run region
        let run_ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                self.vcpu_mmap_size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd.as_raw_fd(),
                0,
            )
        };
        if run_ptr == libc::MAP_FAILED {
            bail!(
                "mmap kvm_run failed: {}",
                std::io::Error::last_os_error()
            );
        }

        Ok(VcpuFd {
            fd,
            run: run_ptr as *mut u8,
            run_size: self.vcpu_mmap_size,
            id,
        })
    }

    /// Query the preferred aarch64 CPU target.
    #[cfg(target_arch = "aarch64")]
    pub fn preferred_target(&self) -> Result<KvmVcpuInit> {
        let mut init = KvmVcpuInit {
            target: 0,
            features: [0; 7],
        };
        let ret = unsafe {
            libc::ioctl(
                self.fd.as_raw_fd(),
                KVM_ARM_PREFERRED_TARGET as libc::c_ulong,
                &mut init as *mut _ as u64,
            )
        };
        if ret < 0 {
            bail!(
                "KVM_ARM_PREFERRED_TARGET failed: {}",
                std::io::Error::last_os_error()
            );
        }
        Ok(init)
    }

    /// Create an in-kernel GICv3 interrupt controller.
    #[cfg(target_arch = "aarch64")]
    pub fn create_gic(&self, cpu_count: u32) -> Result<OwnedFd> {
        use super::memory::{GIC_DIST_BASE, GIC_REDIST_BASE};

        let mut dev = KvmCreateDevice {
            type_: KVM_DEV_TYPE_ARM_VGIC_V3,
            fd: 0,
            flags: 0,
        };
        let ret = unsafe {
            libc::ioctl(
                self.fd.as_raw_fd(),
                KVM_CREATE_DEVICE as libc::c_ulong,
                &mut dev as *mut _ as u64,
            )
        };
        if ret < 0 {
            bail!(
                "KVM_CREATE_DEVICE(GICv3) failed: {}",
                std::io::Error::last_os_error()
            );
        }
        let gic_fd = unsafe { OwnedFd::from_raw_fd(dev.fd as RawFd) };

        // Set distributor address
        let mut dist_addr: u64 = GIC_DIST_BASE;
        set_device_attr(
            gic_fd.as_raw_fd(),
            KVM_DEV_ARM_VGIC_GRP_ADDR,
            KVM_VGIC_V3_ADDR_TYPE_DIST,
            &mut dist_addr as *mut u64 as u64,
        )?;

        // Set redistributor address
        let mut redist_addr: u64 = GIC_REDIST_BASE;
        set_device_attr(
            gic_fd.as_raw_fd(),
            KVM_DEV_ARM_VGIC_GRP_ADDR,
            KVM_VGIC_V3_ADDR_TYPE_REDIST,
            &mut redist_addr as *mut u64 as u64,
        )?;

        // Set number of IRQs (minimum 64, must be multiple of 32)
        let nr_irqs: u32 = 128;
        set_device_attr(
            gic_fd.as_raw_fd(),
            KVM_DEV_ARM_VGIC_GRP_NR_IRQS,
            0,
            &nr_irqs as *const u32 as u64,
        )?;

        // Finalize GIC initialization
        set_device_attr(
            gic_fd.as_raw_fd(),
            KVM_DEV_ARM_VGIC_GRP_CTRL,
            KVM_DEV_ARM_VGIC_CTRL_INIT,
            0,
        )?;

        Ok(gic_fd)
    }

    /// Bind an eventfd to a GSI (interrupt line) via KVM_IRQFD.
    pub fn irqfd(&self, eventfd: RawFd, gsi: u32) -> Result<()> {
        let irqfd = KvmIrqfd {
            fd: eventfd as u32,
            gsi,
            flags: 0,
            resamplefd: 0,
            pad: [0; 16],
        };
        let ret = unsafe {
            libc::ioctl(
                self.fd.as_raw_fd(),
                KVM_IRQFD as libc::c_ulong,
                &irqfd as *const _ as u64,
            )
        };
        if ret < 0 {
            bail!(
                "KVM_IRQFD(gsi={gsi}) failed: {}",
                std::io::Error::last_os_error()
            );
        }
        Ok(())
    }
}

#[cfg(target_arch = "aarch64")]
fn set_device_attr(dev_fd: RawFd, group: u32, attr: u64, addr: u64) -> Result<()> {
    let kda = KvmDeviceAttr {
        flags: 0,
        group,
        attr,
        addr,
    };
    let ret = unsafe {
        libc::ioctl(
            dev_fd,
            KVM_SET_DEVICE_ATTR as libc::c_ulong,
            &kda as *const _ as u64,
        )
    };
    if ret < 0 {
        bail!(
            "KVM_SET_DEVICE_ATTR(group={group}, attr={attr}) failed: {}",
            std::io::Error::last_os_error()
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// VcpuFd: vCPU file descriptor with mmap'd kvm_run
// ---------------------------------------------------------------------------

/// Owned handle to a KVM vCPU with mmap'd run region.
pub(super) struct VcpuFd {
    fd: OwnedFd,
    run: *mut u8,
    run_size: usize,
    id: u32,
}

// Safety: the mmap'd run region is only accessed by the vCPU's own thread
// during KVM_RUN, or after KVM_RUN returns. We enforce single-writer via
// the run loop structure.
unsafe impl Send for VcpuFd {}

impl VcpuFd {
    pub fn id(&self) -> u32 {
        self.id
    }

    /// Initialize the vCPU with the preferred target.
    #[cfg(target_arch = "aarch64")]
    pub fn vcpu_init(&self, preferred: &KvmVcpuInit, power_off: bool) -> Result<()> {
        let mut init = preferred.clone();
        init.features[0] |= 1 << KVM_ARM_VCPU_PSCI_0_2;
        if power_off {
            init.features[0] |= 1 << KVM_ARM_VCPU_POWER_OFF;
        }
        let ret = unsafe {
            libc::ioctl(
                self.fd.as_raw_fd(),
                KVM_ARM_VCPU_INIT as libc::c_ulong,
                &init as *const _ as u64,
            )
        };
        if ret < 0 {
            bail!(
                "KVM_ARM_VCPU_INIT failed: {}",
                std::io::Error::last_os_error()
            );
        }
        Ok(())
    }

    /// Set a single register value.
    #[cfg(target_arch = "aarch64")]
    pub fn set_one_reg(&self, reg_id: u64, value: u64) -> Result<()> {
        let mut val = value;
        let reg = KvmOneReg {
            id: reg_id,
            addr: &mut val as *mut u64 as u64,
        };
        let ret = unsafe {
            libc::ioctl(
                self.fd.as_raw_fd(),
                KVM_SET_ONE_REG as libc::c_ulong,
                &reg as *const _ as u64,
            )
        };
        if ret < 0 {
            bail!(
                "KVM_SET_ONE_REG(0x{:x}) failed: {}",
                reg_id,
                std::io::Error::last_os_error()
            );
        }
        Ok(())
    }

    /// Run the vCPU. Returns the exit reason.
    pub fn run(&self) -> Result<VcpuExit> {
        let ret = unsafe {
            libc::ioctl(
                self.fd.as_raw_fd(),
                KVM_RUN as libc::c_ulong,
                0u64,
            )
        };
        if ret < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                return Ok(VcpuExit::Interrupted);
            }
            bail!("KVM_RUN failed: {}", err);
        }

        // Read exit reason from mmap'd kvm_run
        let exit_reason = unsafe {
            *(self.run.add(KVM_RUN_EXIT_REASON_OFFSET) as *const u32)
        };

        match exit_reason {
            KVM_EXIT_MMIO => {
                let mmio = unsafe {
                    &*(self.run.add(KVM_RUN_EXIT_DATA_OFFSET) as *const KvmRunMmio)
                };
                Ok(VcpuExit::Mmio {
                    addr: mmio.phys_addr,
                    data_offset: KVM_RUN_EXIT_DATA_OFFSET + 8, // offset of data field in kvm_run
                    len: mmio.len,
                    is_write: mmio.is_write != 0,
                })
            }
            KVM_EXIT_SYSTEM_EVENT => {
                let event = unsafe {
                    &*(self.run.add(KVM_RUN_EXIT_DATA_OFFSET) as *const KvmRunSystemEvent)
                };
                Ok(VcpuExit::SystemEvent {
                    event_type: event.type_,
                })
            }
            #[cfg(target_arch = "x86_64")]
            KVM_EXIT_IO => {
                let io = unsafe {
                    &*(self.run.add(KVM_RUN_EXIT_DATA_OFFSET) as *const KvmRunIo)
                };
                Ok(VcpuExit::Io {
                    direction: io.direction,
                    port: io.port,
                    size: io.size,
                })
            }
            #[cfg(target_arch = "x86_64")]
            KVM_EXIT_HLT => Ok(VcpuExit::Hlt),
            #[cfg(target_arch = "x86_64")]
            KVM_EXIT_SHUTDOWN => Ok(VcpuExit::Shutdown),
            KVM_EXIT_INTERNAL_ERROR => {
                Ok(VcpuExit::InternalError)
            }
            other => {
                Ok(VcpuExit::Unknown(other))
            }
        }
    }

    /// Get a mutable pointer to the kvm_run MMIO data buffer.
    /// Used by the MMIO handler to write read responses back.
    pub fn mmio_data_mut(&self) -> &mut [u8; 8] {
        unsafe {
            &mut *(self.run.add(KVM_RUN_EXIT_DATA_OFFSET + 8) as *mut [u8; 8])
        }
    }
}

impl Drop for VcpuFd {
    fn drop(&mut self) {
        if !self.run.is_null() {
            unsafe {
                libc::munmap(self.run as *mut libc::c_void, self.run_size);
            }
        }
    }
}

/// vCPU exit reasons.
#[derive(Debug)]
pub(super) enum VcpuExit {
    Mmio {
        addr: u64,
        data_offset: usize,
        len: u32,
        is_write: bool,
    },
    #[cfg(target_arch = "x86_64")]
    Io {
        direction: u8,
        port: u16,
        size: u8,
    },
    SystemEvent {
        event_type: u32,
    },
    #[cfg(target_arch = "x86_64")]
    Hlt,
    #[cfg(target_arch = "x86_64")]
    Shutdown,
    InternalError,
    Interrupted,
    Unknown(u32),
}

// ---------------------------------------------------------------------------
// x86_64-specific ioctl numbers
// ---------------------------------------------------------------------------

#[cfg(target_arch = "x86_64")]
pub(super) const KVM_SET_TSS_ADDR: u64 = _io(KVMIO, 0x47);
#[cfg(target_arch = "x86_64")]
pub(super) const KVM_SET_IDENTITY_MAP_ADDR: u64 = _iow(KVMIO, 0x48, 8);
#[cfg(target_arch = "x86_64")]
pub(super) const KVM_CREATE_IRQCHIP: u64 = _io(KVMIO, 0x60);
#[cfg(target_arch = "x86_64")]
pub(super) const KVM_CREATE_PIT2: u64 = _iow(KVMIO, 0x77, 68); // sizeof kvm_pit_config
#[cfg(target_arch = "x86_64")]
pub(super) const KVM_SET_REGS: u64 = _iow(KVMIO, 0x82, 144); // sizeof kvm_regs
#[cfg(target_arch = "x86_64")]
pub(super) const KVM_SET_SREGS: u64 = _iow(KVMIO, 0x84, 312); // sizeof kvm_sregs
#[cfg(target_arch = "x86_64")]
pub(super) const KVM_GET_SUPPORTED_CPUID: u64 = _iowr(KVMIO, 0x05, 8); // sizeof kvm_cpuid2 header

// ---------------------------------------------------------------------------
// x86_64 exit reasons
// ---------------------------------------------------------------------------

#[cfg(target_arch = "x86_64")]
pub(super) const KVM_EXIT_IO: u32 = 2;
#[cfg(target_arch = "x86_64")]
pub(super) const KVM_EXIT_HLT: u32 = 5;
#[cfg(target_arch = "x86_64")]
pub(super) const KVM_EXIT_SHUTDOWN: u32 = 8;

// ---------------------------------------------------------------------------
// x86_64 repr(C) structs
// ---------------------------------------------------------------------------

#[cfg(target_arch = "x86_64")]
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct KvmRegs {
    pub rax: u64, pub rbx: u64, pub rcx: u64, pub rdx: u64,
    pub rsi: u64, pub rdi: u64, pub rsp: u64, pub rbp: u64,
    pub r8: u64,  pub r9: u64,  pub r10: u64, pub r11: u64,
    pub r12: u64, pub r13: u64, pub r14: u64, pub r15: u64,
    pub rip: u64,
    pub rflags: u64,
}

#[cfg(target_arch = "x86_64")]
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct KvmSegment {
    pub base: u64,
    pub limit: u32,
    pub selector: u16,
    pub type_: u8,
    pub present: u8,
    pub dpl: u8,
    pub db: u8,
    pub s: u8,
    pub l: u8,
    pub g: u8,
    pub avl: u8,
    pub unusable: u8,
    pub padding: u8,
}

#[cfg(target_arch = "x86_64")]
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct KvmDtable {
    pub base: u64,
    pub limit: u16,
    pub padding: [u16; 3],
}

#[cfg(target_arch = "x86_64")]
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct KvmSregs {
    pub cs: KvmSegment,
    pub ds: KvmSegment,
    pub es: KvmSegment,
    pub fs: KvmSegment,
    pub gs: KvmSegment,
    pub ss: KvmSegment,
    pub tr: KvmSegment,
    pub ldt: KvmSegment,
    pub gdt: KvmDtable,
    pub idt: KvmDtable,
    pub cr0: u64,
    pub cr2: u64,
    pub cr3: u64,
    pub cr4: u64,
    pub cr8: u64,
    pub efer: u64,
    pub apic_base: u64,
    pub interrupt_bitmap: [u64; 4],
}

#[cfg(target_arch = "x86_64")]
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(super) struct KvmCpuidEntry2 {
    pub function: u32,
    pub index: u32,
    pub flags: u32,
    pub eax: u32,
    pub ebx: u32,
    pub ecx: u32,
    pub edx: u32,
    pub padding: [u32; 3],
}

/// Header for KVM_GET_SUPPORTED_CPUID / KVM_SET_CPUID2.
/// Followed by `nent` KvmCpuidEntry2 structs.
#[cfg(target_arch = "x86_64")]
#[repr(C)]
pub(super) struct KvmCpuid2 {
    pub nent: u32,
    pub padding: u32,
    pub entries: [KvmCpuidEntry2; 0], // flexible array
}

#[cfg(target_arch = "x86_64")]
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct KvmPitConfig {
    pub flags: u32,
    pub pad: [u32; 15],
}

/// kvm_run IO exit data (at offset 32 in the kvm_run mmap'd region).
#[cfg(target_arch = "x86_64")]
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(super) struct KvmRunIo {
    pub direction: u8,
    pub size: u8,
    pub port: u16,
    pub count: u32,
    pub data_offset: u64,
}

#[cfg(target_arch = "x86_64")]
const _: () = {
    assert!(std::mem::size_of::<KvmRegs>() == 144);
    assert!(std::mem::size_of::<KvmSregs>() == 312);
    assert!(std::mem::size_of::<KvmSegment>() == 24);
    assert!(std::mem::size_of::<KvmDtable>() == 16);
    assert!(std::mem::size_of::<KvmPitConfig>() == 64);
    assert!(std::mem::size_of::<KvmCpuidEntry2>() == 40);
};

// ---------------------------------------------------------------------------
// x86_64 VmFd methods
// ---------------------------------------------------------------------------

#[cfg(target_arch = "x86_64")]
impl VmFd {
    /// Set the TSS address (required before creating IRQCHIP on x86_64).
    pub fn set_tss_addr(&self, addr: u64) -> Result<()> {
        let ret = unsafe {
            libc::ioctl(self.fd.as_raw_fd(), KVM_SET_TSS_ADDR as libc::c_ulong, addr)
        };
        if ret < 0 {
            bail!("KVM_SET_TSS_ADDR failed: {}", std::io::Error::last_os_error());
        }
        Ok(())
    }

    /// Set the identity map address.
    pub fn set_identity_map_addr(&self, addr: u64) -> Result<()> {
        let ret = unsafe {
            libc::ioctl(
                self.fd.as_raw_fd(),
                KVM_SET_IDENTITY_MAP_ADDR as libc::c_ulong,
                &addr as *const u64 as u64,
            )
        };
        if ret < 0 {
            bail!("KVM_SET_IDENTITY_MAP_ADDR failed: {}", std::io::Error::last_os_error());
        }
        Ok(())
    }

    /// Create an in-kernel i8259 PIC + IOAPIC + LAPIC.
    pub fn create_irqchip(&self) -> Result<()> {
        let ret = unsafe {
            libc::ioctl(self.fd.as_raw_fd(), KVM_CREATE_IRQCHIP as libc::c_ulong, 0u64)
        };
        if ret < 0 {
            bail!("KVM_CREATE_IRQCHIP failed: {}", std::io::Error::last_os_error());
        }
        Ok(())
    }

    /// Create an in-kernel i8254 PIT.
    pub fn create_pit2(&self) -> Result<()> {
        let config = KvmPitConfig::default();
        let ret = unsafe {
            libc::ioctl(
                self.fd.as_raw_fd(),
                KVM_CREATE_PIT2 as libc::c_ulong,
                &config as *const _ as u64,
            )
        };
        if ret < 0 {
            bail!("KVM_CREATE_PIT2 failed: {}", std::io::Error::last_os_error());
        }
        Ok(())
    }

    /// Get CPUID entries supported by this KVM host.
    pub fn get_supported_cpuid(&self) -> Result<Vec<KvmCpuidEntry2>> {
        const MAX_ENTRIES: usize = 256;
        let entry_size = std::mem::size_of::<KvmCpuidEntry2>();
        let header_size = std::mem::size_of::<u32>() * 2; // nent + padding
        let total_size = header_size + MAX_ENTRIES * entry_size;

        let layout = std::alloc::Layout::from_size_align(total_size, 8)
            .context("cpuid layout")?;
        let buf = unsafe { std::alloc::alloc_zeroed(layout) };
        if buf.is_null() {
            bail!("failed to allocate CPUID buffer");
        }

        // Set nent to MAX_ENTRIES
        unsafe { *(buf as *mut u32) = MAX_ENTRIES as u32; }

        let ret = unsafe {
            libc::ioctl(
                self.fd.as_raw_fd(),
                KVM_GET_SUPPORTED_CPUID as libc::c_ulong,
                buf as u64,
            )
        };
        if ret < 0 {
            unsafe { std::alloc::dealloc(buf, layout); }
            bail!("KVM_GET_SUPPORTED_CPUID failed: {}", std::io::Error::last_os_error());
        }

        let nent = unsafe { *(buf as *const u32) } as usize;
        let entries_ptr = unsafe { buf.add(header_size) as *const KvmCpuidEntry2 };
        let entries = unsafe { std::slice::from_raw_parts(entries_ptr, nent) }.to_vec();
        unsafe { std::alloc::dealloc(buf, layout); }
        Ok(entries)
    }
}

// ---------------------------------------------------------------------------
// x86_64 VcpuFd methods
// ---------------------------------------------------------------------------

#[cfg(target_arch = "x86_64")]
impl VcpuFd {
    /// Set general-purpose registers.
    pub fn set_regs(&self, regs: &KvmRegs) -> Result<()> {
        let ret = unsafe {
            libc::ioctl(
                self.fd.as_raw_fd(),
                KVM_SET_REGS as libc::c_ulong,
                regs as *const _ as u64,
            )
        };
        if ret < 0 {
            bail!("KVM_SET_REGS failed: {}", std::io::Error::last_os_error());
        }
        Ok(())
    }

    /// Set special registers (segments, control registers, EFER).
    pub fn set_sregs(&self, sregs: &KvmSregs) -> Result<()> {
        let ret = unsafe {
            libc::ioctl(
                self.fd.as_raw_fd(),
                KVM_SET_SREGS as libc::c_ulong,
                sregs as *const _ as u64,
            )
        };
        if ret < 0 {
            bail!("KVM_SET_SREGS failed: {}", std::io::Error::last_os_error());
        }
        Ok(())
    }

    /// Set CPUID entries for this vCPU.
    pub fn set_cpuid2(&self, entries: &[KvmCpuidEntry2]) -> Result<()> {
        let entry_size = std::mem::size_of::<KvmCpuidEntry2>();
        let header_size = std::mem::size_of::<u32>() * 2;
        let total_size = header_size + entries.len() * entry_size;

        let layout = std::alloc::Layout::from_size_align(total_size, 8)
            .context("cpuid layout")?;
        let buf = unsafe { std::alloc::alloc_zeroed(layout) };
        if buf.is_null() {
            bail!("failed to allocate CPUID buffer");
        }

        unsafe {
            *(buf as *mut u32) = entries.len() as u32;
            let dst = buf.add(header_size) as *mut KvmCpuidEntry2;
            std::ptr::copy_nonoverlapping(entries.as_ptr(), dst, entries.len());
        }

        // KVM_SET_CPUID2 uses the same ioctl number encoding as GET but with _IOW
        const KVM_SET_CPUID2: u64 = _iow(KVMIO, 0x90, 8);
        let ret = unsafe {
            libc::ioctl(
                self.fd.as_raw_fd(),
                KVM_SET_CPUID2 as libc::c_ulong,
                buf as u64,
            )
        };
        unsafe { std::alloc::dealloc(buf, layout); }
        if ret < 0 {
            bail!("KVM_SET_CPUID2 failed: {}", std::io::Error::last_os_error());
        }
        Ok(())
    }

    /// Get the IO exit data from the kvm_run mmap'd region.
    pub fn io_data(&self) -> &KvmRunIo {
        unsafe {
            &*(self.run.add(KVM_RUN_EXIT_DATA_OFFSET) as *const KvmRunIo)
        }
    }

    /// Get a mutable pointer to the IO data buffer.
    /// The data buffer is at the offset specified in KvmRunIo.data_offset.
    pub fn io_data_mut(&self, data_offset: u64) -> *mut u8 {
        unsafe { self.run.add(data_offset as usize) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // ioctl encoding correctness
    // -----------------------------------------------------------------------

    #[test]
    fn io_encoding() {
        // _IO(0xAE, 0x00) should be 0x0000AE00
        assert_eq!(_io(0xAE, 0x00), 0x0000_AE00);
        assert_eq!(_io(0xAE, 0x01), 0x0000_AE01);
        assert_eq!(_io(0xAE, 0x80), 0x0000_AE80);
    }

    #[test]
    fn iow_encoding() {
        // _IOW has direction bit 30 set
        let val = _iow(0xAE, 0x46, 32);
        assert_eq!(val & 0xFF, 0x46); // nr
        assert_eq!((val >> 8) & 0xFF, 0xAE); // type
        assert_eq!((val >> 16) & 0x3FFF, 32); // size
        assert_ne!(val & (1 << 30), 0); // write direction
        assert_eq!(val & (1 << 31), 0); // not read direction
    }

    #[test]
    fn ior_encoding() {
        let val = _ior(0xAE, 0xAF, 36);
        assert_eq!(val & 0xFF, 0xAF);
        assert_eq!((val >> 8) & 0xFF, 0xAE);
        assert_eq!((val >> 16) & 0x3FFF, 36);
        assert_eq!(val & (1 << 30), 0); // not write
        assert_ne!(val & (1 << 31), 0); // read direction
    }

    #[test]
    fn iowr_encoding() {
        let val = _iowr(0xAE, 0xE0, 12);
        assert_eq!(val & 0xFF, 0xE0);
        assert_ne!(val & (1 << 30), 0); // write
        assert_ne!(val & (1 << 31), 0); // read
    }

    // -----------------------------------------------------------------------
    // Known ioctl number values
    // -----------------------------------------------------------------------

    #[test]
    fn kvm_get_api_version_value() {
        assert_eq!(KVM_GET_API_VERSION, 0x0000_AE00);
    }

    #[test]
    fn kvm_create_vm_value() {
        assert_eq!(KVM_CREATE_VM, 0x0000_AE01);
    }

    #[test]
    fn kvm_check_extension_value() {
        assert_eq!(KVM_CHECK_EXTENSION, 0x0000_AE03);
    }

    #[test]
    fn kvm_run_value() {
        assert_eq!(KVM_RUN, 0x0000_AE80);
    }

    #[test]
    fn kvm_create_vcpu_value() {
        assert_eq!(KVM_CREATE_VCPU, 0x0000_AE41);
    }

    // -----------------------------------------------------------------------
    // struct sizes match kernel expectations
    // -----------------------------------------------------------------------

    #[test]
    fn struct_sizes() {
        assert_eq!(
            std::mem::size_of::<KvmUserspaceMemoryRegion>(),
            32,
            "KvmUserspaceMemoryRegion"
        );
        assert_eq!(
            std::mem::size_of::<KvmCreateDevice>(),
            12,
            "KvmCreateDevice"
        );
        assert_eq!(
            std::mem::size_of::<KvmDeviceAttr>(),
            24,
            "KvmDeviceAttr"
        );
        assert_eq!(
            std::mem::size_of::<KvmIrqfd>(),
            32,
            "KvmIrqfd"
        );
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn struct_sizes_aarch64() {
        assert_eq!(
            std::mem::size_of::<KvmOneReg>(),
            16,
            "KvmOneReg"
        );
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn kvm_vcpu_init_size() {
        let size = std::mem::size_of::<KvmVcpuInit>();
        assert!(size == 32, "KvmVcpuInit size is {size}, expected 32");
    }

    // -----------------------------------------------------------------------
    // ARM64 register ID encoding (aarch64 only)
    // -----------------------------------------------------------------------

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn reg_x0_encoding() {
        assert_eq!(REG_X0, 0x6030_0000_0010_0000);
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn reg_pc_encoding() {
        assert_eq!(REG_PC, 0x6030_0000_0010_0040);
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn reg_pstate_encoding() {
        assert_eq!(REG_PSTATE, 0x6030_0000_0010_0042);
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn reg_x_sequential() {
        assert_eq!(REG_X1 - REG_X0, 2);
        assert_eq!(REG_X2 - REG_X1, 2);
        assert_eq!(REG_X3 - REG_X2, 2);
    }

    // -----------------------------------------------------------------------
    // VcpuExit debug formatting
    // -----------------------------------------------------------------------

    #[test]
    fn vcpu_exit_debug_format() {
        let exit = VcpuExit::Mmio {
            addr: 0x0A00_0000,
            data_offset: 40,
            len: 4,
            is_write: true,
        };
        let s = format!("{exit:?}");
        assert!(s.contains("Mmio"));
        assert!(s.contains("167772160")); // 0x0A000000

        let exit = VcpuExit::SystemEvent { event_type: 1 };
        assert!(format!("{exit:?}").contains("SystemEvent"));
    }

    // -----------------------------------------------------------------------
    // Constants sanity checks
    // -----------------------------------------------------------------------

    #[test]
    fn exit_reason_values() {
        assert_eq!(KVM_EXIT_UNKNOWN, 0);
        assert_eq!(KVM_EXIT_MMIO, 6);
        assert_eq!(KVM_EXIT_SYSTEM_EVENT, 24);
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn gic_constants() {
        assert_eq!(KVM_DEV_TYPE_ARM_VGIC_V3, 5);
        assert_eq!(KVM_DEV_ARM_VGIC_GRP_ADDR, 0);
        assert_eq!(KVM_DEV_ARM_VGIC_GRP_CTRL, 4);
    }

    // -----------------------------------------------------------------------
    // Vhost ioctl constant values
    // -----------------------------------------------------------------------

    #[test]
    fn vhost_set_owner_value() {
        // _IO(0xAF, 0x01) = 0x0000_AF01
        assert_eq!(VHOST_SET_OWNER, 0x0000_AF01);
    }

    #[test]
    fn vhost_set_mem_table_value() {
        // _IOW(0xAF, 0x03, 8)
        let val = VHOST_SET_MEM_TABLE;
        assert_eq!(val & 0xFF, 0x03);
        assert_eq!((val >> 8) & 0xFF, 0xAF);
        assert_eq!((val >> 16) & 0x3FFF, 8);
        assert_ne!(val & (1 << 30), 0); // write direction
    }

    #[test]
    fn vhost_set_vring_num_value() {
        let val = VHOST_SET_VRING_NUM;
        assert_eq!(val & 0xFF, 0x10);
        assert_eq!((val >> 8) & 0xFF, 0xAF);
        assert_eq!((val >> 16) & 0x3FFF, 8);
    }

    #[test]
    fn vhost_set_vring_addr_value() {
        let val = VHOST_SET_VRING_ADDR;
        assert_eq!(val & 0xFF, 0x11);
        assert_eq!((val >> 8) & 0xFF, 0xAF);
        assert_eq!((val >> 16) & 0x3FFF, 48);
    }

    #[test]
    fn vhost_vsock_set_guest_cid_value() {
        let val = VHOST_VSOCK_SET_GUEST_CID;
        assert_eq!(val & 0xFF, 0x60);
        assert_eq!((val >> 8) & 0xFF, 0xAF);
        assert_eq!((val >> 16) & 0x3FFF, 8);
    }

    #[test]
    fn vhost_kick_call_values() {
        let kick = VHOST_SET_VRING_KICK;
        assert_eq!(kick & 0xFF, 0x20);
        let call = VHOST_SET_VRING_CALL;
        assert_eq!(call & 0xFF, 0x21);
    }

    // -----------------------------------------------------------------------
    // Vhost struct sizes
    // -----------------------------------------------------------------------

    #[test]
    fn vhost_struct_sizes() {
        assert_eq!(std::mem::size_of::<VhostVringState>(), 8, "VhostVringState");
        assert_eq!(std::mem::size_of::<VhostVringAddr>(), 48, "VhostVringAddr");
        assert_eq!(std::mem::size_of::<VhostVringFile>(), 8, "VhostVringFile");
        assert_eq!(std::mem::size_of::<VhostMemoryRegion>(), 32, "VhostMemoryRegion");
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn pstate_el1h_value() {
        assert_eq!(PSTATE_EL1H_DAIF, 0x3C5);
        assert_eq!(PSTATE_EL1H_DAIF & 0x1F, 5);
        assert_ne!(PSTATE_EL1H_DAIF & (1 << 6), 0); // F
        assert_ne!(PSTATE_EL1H_DAIF & (1 << 7), 0); // I
        assert_ne!(PSTATE_EL1H_DAIF & (1 << 8), 0); // A
        assert_ne!(PSTATE_EL1H_DAIF & (1 << 9), 0); // D
    }

    // -----------------------------------------------------------------------
    // VcpuFd is Send
    // -----------------------------------------------------------------------

    #[test]
    fn vcpu_fd_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<VcpuFd>();
    }

    // -----------------------------------------------------------------------
    // /dev/kvm tests (skip on macOS)
    // -----------------------------------------------------------------------

    fn require_kvm() -> Option<KvmFd> {
        match KvmFd::open() {
            Ok(kvm) => Some(kvm),
            Err(_) => {
                eprintln!("SKIPPED: /dev/kvm not available");
                None
            }
        }
    }

    #[test]
    fn kvm_open_and_version() {
        let Some(kvm) = require_kvm() else { return };
        // If we got here, API version was already verified as 12
        let _ = kvm;
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn kvm_check_one_reg_extension() {
        let Some(kvm) = require_kvm() else { return };
        let val = kvm.check_extension(KVM_CAP_ONE_REG).unwrap();
        assert!(val > 0, "KVM_CAP_ONE_REG should be supported");
    }

    #[test]
    fn kvm_check_irqfd_extension() {
        let Some(kvm) = require_kvm() else { return };
        let val = kvm.check_extension(KVM_CAP_IRQFD).unwrap();
        assert!(val > 0, "KVM_CAP_IRQFD should be supported");
    }

    #[test]
    fn kvm_create_vm_succeeds() {
        let Some(kvm) = require_kvm() else { return };
        let vm = kvm.create_vm();
        assert!(vm.is_ok(), "create_vm failed: {:?}", vm.err());
    }

    #[test]
    fn kvm_create_vcpu_succeeds() {
        let Some(kvm) = require_kvm() else { return };
        let vm = kvm.create_vm().unwrap();
        let vcpu = vm.create_vcpu(0);
        assert!(vcpu.is_ok(), "create_vcpu failed: {:?}", vcpu.err());
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn kvm_preferred_target() {
        let Some(kvm) = require_kvm() else { return };
        let vm = kvm.create_vm().unwrap();
        let target = vm.preferred_target();
        assert!(target.is_ok(), "preferred_target failed: {:?}", target.err());
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn kvm_vcpu_init_succeeds() {
        let Some(kvm) = require_kvm() else { return };
        let vm = kvm.create_vm().unwrap();
        let vcpu = vm.create_vcpu(0).unwrap();
        let target = vm.preferred_target().unwrap();
        let result = vcpu.vcpu_init(&target, false);
        assert!(result.is_ok(), "vcpu_init failed: {:?}", result.err());
    }

    #[test]
    fn kvm_set_memory_region() {
        let Some(kvm) = require_kvm() else { return };
        let vm = kvm.create_vm().unwrap();

        // Allocate a page of memory
        let page_size = 4096usize;
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                page_size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        assert_ne!(ptr, libc::MAP_FAILED);

        let result = vm.set_user_memory_region(0, 0x4000_0000, page_size as u64, ptr as *const u8);
        assert!(result.is_ok(), "set_user_memory_region failed: {:?}", result.err());

        unsafe { libc::munmap(ptr, page_size); }
    }

    // -----------------------------------------------------------------------
    // x86_64 struct sizes
    // -----------------------------------------------------------------------

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn struct_sizes_x86_64() {
        assert_eq!(std::mem::size_of::<KvmRegs>(), 144, "KvmRegs");
        assert_eq!(std::mem::size_of::<KvmSegment>(), 24, "KvmSegment");
        assert_eq!(std::mem::size_of::<KvmDtable>(), 16, "KvmDtable");
        assert_eq!(std::mem::size_of::<KvmSregs>(), 312, "KvmSregs");
        assert_eq!(std::mem::size_of::<KvmPitConfig>(), 64, "KvmPitConfig");
        assert_eq!(std::mem::size_of::<KvmCpuidEntry2>(), 40, "KvmCpuidEntry2");
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn x86_64_exit_reason_values() {
        assert_eq!(KVM_EXIT_IO, 2);
        assert_eq!(KVM_EXIT_HLT, 5);
        assert_eq!(KVM_EXIT_SHUTDOWN, 8);
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn kvm_x86_64_create_irqchip() {
        let Some(kvm) = require_kvm() else { return };
        let vm = kvm.create_vm().unwrap();
        vm.set_tss_addr(0xFFFB_D000).unwrap();
        vm.set_identity_map_addr(0xFFFB_C000).unwrap();
        vm.create_irqchip().unwrap();
        // PIT may not be available in nested KVM / CI environments
        let _ = vm.create_pit2();
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn kvm_x86_64_get_supported_cpuid() {
        let Some(kvm) = require_kvm() else { return };
        let vm = kvm.create_vm().unwrap();
        let entries = vm.get_supported_cpuid().unwrap();
        assert!(!entries.is_empty(), "should have CPUID entries");
    }
}
