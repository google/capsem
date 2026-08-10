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

#[cfg(target_arch = "x86_64")]
#[test]
fn kvm_x86_64_checkpoint_ioctl_values() {
    assert_eq!(KVM_GET_LAPIC, 0x8400_AE8E);
    assert_eq!(KVM_SET_LAPIC, 0x4400_AE8F);
    assert_eq!(KVM_GET_IRQCHIP, 0xC208_AE62);
    assert_eq!(KVM_SET_IRQCHIP, 0x8208_AE63);
    assert_eq!(KVM_GET_PIT2, 0x8070_AE9F);
    assert_eq!(KVM_SET_PIT2, 0x4070_AEA0);
    assert_eq!(KVM_GET_CLOCK, 0x8030_AE7C);
    assert_eq!(KVM_SET_CLOCK, 0x4030_AE7B);
    assert_eq!(KVM_GET_MSRS, 0xC008_AE88);
    assert_eq!(KVM_SET_MSRS, 0x4008_AE89);
    assert_eq!(KVM_GET_VCPU_EVENTS, 0x8040_AE9F);
    assert_eq!(KVM_SET_VCPU_EVENTS, 0x4040_AEA0);
    assert_eq!(KVM_GET_FPU, 0x81A0_AE8C);
    assert_eq!(KVM_SET_FPU, 0x41A0_AE8D);
    assert_eq!(KVM_GET_XCRS, 0x8188_AEA6);
    assert_eq!(KVM_SET_XCRS, 0x4188_AEA7);
    assert_eq!(KVM_GET_XSAVE, 0x9000_AEA4);
    assert_eq!(KVM_SET_XSAVE, 0x5000_AEA5);
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
    assert_eq!(std::mem::size_of::<KvmDeviceAttr>(), 24, "KvmDeviceAttr");
    assert_eq!(std::mem::size_of::<KvmIrqfd>(), 32, "KvmIrqfd");
}

#[cfg(target_arch = "aarch64")]
#[test]
fn struct_sizes_aarch64() {
    assert_eq!(std::mem::size_of::<KvmOneReg>(), 16, "KvmOneReg");
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

#[test]
fn kvm_run_eagain_is_transient_not_ready() {
    let err = std::io::Error::from_raw_os_error(libc::EAGAIN);
    assert!(matches!(
        classify_kvm_run_error(&err),
        Some(VcpuExit::NotReady)
    ));
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
    assert_eq!((val >> 16) & 0x3FFF, 40);
}

#[test]
fn vhost_get_vring_base_value() {
    assert_eq!(VHOST_GET_VRING_BASE, 0xC008_AF12);
    assert_ne!(VHOST_GET_VRING_BASE & (1 << 30), 0); // write direction
    assert_ne!(VHOST_GET_VRING_BASE & (1 << 31), 0); // read direction
}

#[test]
fn vhost_features_values() {
    let get = VHOST_GET_FEATURES;
    assert_eq!(get & 0xFF, 0x00);
    assert_eq!((get >> 8) & 0xFF, 0xAF);
    assert_eq!((get >> 16) & 0x3FFF, 8);

    let set = VHOST_SET_FEATURES;
    assert_eq!(set & 0xFF, 0x00);
    assert_eq!((set >> 8) & 0xFF, 0xAF);
    assert_eq!((set >> 16) & 0x3FFF, 8);
}

#[test]
fn vhost_vsock_set_guest_cid_value() {
    let val = VHOST_VSOCK_SET_GUEST_CID;
    assert_eq!(val & 0xFF, 0x60);
    assert_eq!((val >> 8) & 0xFF, 0xAF);
    assert_eq!((val >> 16) & 0x3FFF, 8);
}

#[test]
fn vhost_vsock_set_running_value() {
    let val = VHOST_VSOCK_SET_RUNNING;
    assert_eq!(val & 0xFF, 0x61);
    assert_eq!((val >> 8) & 0xFF, 0xAF);
    assert_eq!((val >> 16) & 0x3FFF, 4);
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
    assert_eq!(std::mem::size_of::<VhostVringAddr>(), 40, "VhostVringAddr");
    assert_eq!(std::mem::size_of::<VhostVringFile>(), 8, "VhostVringFile");
    assert_eq!(
        std::mem::size_of::<VhostMemoryRegion>(),
        32,
        "VhostMemoryRegion"
    );
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
    if std::env::var_os("CAPSEM_SKIP_KVM_TESTS").is_some() {
        eprintln!("SKIPPED: CAPSEM_SKIP_KVM_TESTS set");
        return None;
    }
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
fn kvm_check_ioeventfd_extension() {
    let Some(kvm) = require_kvm() else { return };
    let val = kvm.check_extension(KVM_CAP_IOEVENTFD).unwrap();
    assert!(val > 0, "KVM_CAP_IOEVENTFD should be supported");
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
    assert!(
        target.is_ok(),
        "preferred_target failed: {:?}",
        target.err()
    );
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
    assert!(
        result.is_ok(),
        "set_user_memory_region failed: {:?}",
        result.err()
    );

    unsafe {
        libc::munmap(ptr, page_size);
    }
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
    assert_eq!(std::mem::size_of::<KvmEnableCap>(), 104, "KvmEnableCap");
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
fn x86_64_mp_state_values() {
    assert_eq!(KVM_GET_MP_STATE, 0x8004_AE98);
    assert_eq!(KVM_SET_MP_STATE, 0x4004_AE99);
    assert_eq!(KVM_MP_STATE_RUNNABLE, 0);
    assert_eq!(KVM_MP_STATE_UNINITIALIZED, 1);
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
fn kvm_x86_64_split_irqchip_create_vcpu() {
    let Some(kvm) = require_kvm() else { return };
    if kvm.check_extension(KVM_CAP_SPLIT_IRQCHIP).unwrap_or(0) <= 0 {
        eprintln!("SKIPPED: KVM_CAP_SPLIT_IRQCHIP not supported");
        return;
    }
    let vm = kvm.create_vm().unwrap();
    vm.set_tss_addr(0xFFFB_D000).unwrap();
    vm.set_identity_map_addr(0xFFFB_C000).unwrap();
    vm.enable_split_irqchip(24).unwrap();
    vm.create_vcpu(0).unwrap();
}

#[cfg(target_arch = "x86_64")]
#[test]
fn kvm_x86_64_ap_vcpu_can_be_parked_for_sipi() {
    let Some(kvm) = require_kvm() else { return };
    if kvm.check_extension(KVM_CAP_SPLIT_IRQCHIP).unwrap_or(0) <= 0 {
        eprintln!("SKIPPED: KVM_CAP_SPLIT_IRQCHIP not supported");
        return;
    }
    let vm = kvm.create_vm().unwrap();
    vm.set_tss_addr(0xFFFB_D000).unwrap();
    vm.set_identity_map_addr(0xFFFB_C000).unwrap();
    vm.enable_split_irqchip(24).unwrap();
    let bsp = vm.create_vcpu(0).unwrap();
    let ap = vm.create_vcpu(1).unwrap();

    bsp.set_mp_state(KvmMpState {
        mp_state: KVM_MP_STATE_RUNNABLE,
    })
    .unwrap();
    ap.set_mp_state(KvmMpState {
        mp_state: KVM_MP_STATE_UNINITIALIZED,
    })
    .unwrap();

    assert_eq!(bsp.get_mp_state().unwrap().mp_state, KVM_MP_STATE_RUNNABLE);
    assert_eq!(
        ap.get_mp_state().unwrap().mp_state,
        KVM_MP_STATE_UNINITIALIZED
    );
}

#[cfg(target_arch = "x86_64")]
#[test]
fn kvm_x86_64_large_memory_split_around_pci_hole_create_vcpu() {
    let Some(kvm) = require_kvm() else { return };
    if kvm.check_extension(KVM_CAP_SPLIT_IRQCHIP).unwrap_or(0) <= 0 {
        eprintln!("SKIPPED: KVM_CAP_SPLIT_IRQCHIP not supported");
        return;
    }
    let vm = kvm.create_vm().unwrap();
    let ram_size = 4 * 1024 * 1024 * 1024u64;
    let guest_mem = super::super::memory::GuestMemory::new(ram_size).unwrap();
    for region in super::super::memory::kvm_memory_regions(ram_size) {
        vm.set_user_memory_region(
            region.slot,
            region.guest_phys_addr,
            region.memory_size,
            guest_mem.as_ptr_at(region.host_offset).unwrap(),
        )
        .unwrap();
    }
    vm.set_tss_addr(0xFFFB_D000).unwrap();
    vm.set_identity_map_addr(0xFFFB_C000).unwrap();
    vm.enable_split_irqchip(24).unwrap();
    vm.create_vcpu(0).unwrap();
}

#[cfg(target_arch = "x86_64")]
#[test]
fn kvm_x86_64_get_supported_cpuid() {
    let Some(kvm) = require_kvm() else { return };
    let entries = kvm.get_supported_cpuid().unwrap();
    assert!(!entries.is_empty(), "should have CPUID entries");
}
