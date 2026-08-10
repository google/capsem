use super::*;

fn test_header() -> CheckpointHeader {
    #[cfg(target_arch = "x86_64")]
    let vcpu_state_len = X86_VCPU_STATE_LEN;
    #[cfg(not(target_arch = "x86_64"))]
    let vcpu_state_len = 0;

    CheckpointHeader {
        version: VERSION,
        arch: arch_tag(),
        ram_bytes: 4096,
        vcpu_count: 2,
        vcpu_state_len,
        mmio_device_count: 3,
    }
}

#[cfg(target_arch = "x86_64")]
fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("capsem-kvm-checkpoint")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn header_roundtrips() {
    let header = test_header();
    let decoded = CheckpointHeader::decode(&header.encode()).unwrap();
    assert_eq!(decoded, header);
    assert_eq!(decoded.version, VERSION);
    assert_eq!(decoded.version, 8, "VirtioFS backend state requires v8");
    assert_eq!(decoded.ram_bytes, 4096);
    assert_eq!(decoded.vcpu_count, 2);
    #[cfg(target_arch = "x86_64")]
    assert_eq!(decoded.vcpu_state_len, X86_VCPU_STATE_LEN);
    #[cfg(not(target_arch = "x86_64"))]
    assert_eq!(decoded.vcpu_state_len, 0);
    assert_eq!(decoded.mmio_device_count, 3);
}

#[test]
fn header_rejects_bad_magic() {
    let mut encoded = test_header().encode();
    encoded[0] = b'X';
    let err = CheckpointHeader::decode(&encoded).unwrap_err();
    assert!(err.to_string().contains("bad checkpoint magic"));
}

#[cfg(target_arch = "x86_64")]
fn snapshot(id: u32) -> VcpuSnapshot {
    let regs = KvmRegs {
        rax: id as u64 + 10,
        rip: 0x1000 + id as u64,
        ..Default::default()
    };
    let sregs = KvmSregs {
        cr3: 0x2000 + id as u64,
        ..Default::default()
    };
    let mp_state = KvmMpState {
        mp_state: KVM_MP_STATE_RUNNABLE,
    };
    VcpuSnapshot {
        id,
        regs,
        sregs,
        mp_state,
        msrs: vec![KvmMsrEntry {
            index: 0x6e0,
            reserved: 0,
            data: 0x1000 + id as u64,
        }],
        lapic: KvmLapicState::default(),
        events: KvmVcpuEvents::default(),
        debugregs: KvmDebugRegs::default(),
        fpu: KvmFpu::default(),
        xcrs: KvmXcrs::default(),
        xsave: KvmXsave::default(),
    }
}

#[cfg(target_arch = "x86_64")]
fn vm_snapshot() -> VmSnapshot {
    let mut pic_master = KvmIrqchip {
        chip_id: KVM_IRQCHIP_PIC_MASTER,
        ..Default::default()
    };
    pic_master.chip[0] = 1;
    let mut pic_slave = KvmIrqchip {
        chip_id: KVM_IRQCHIP_PIC_SLAVE,
        ..Default::default()
    };
    pic_slave.chip[0] = 2;
    let mut ioapic = KvmIrqchip {
        chip_id: KVM_IRQCHIP_IOAPIC,
        ..Default::default()
    };
    ioapic.chip[0] = 3;
    let mut pit2 = KvmPitState2::default();
    pit2.bytes[0] = 4;
    let mut clock = KvmClockData::default();
    clock.bytes[0] = 5;
    VmSnapshot {
        irqchips: [pic_master, pic_slave, ioapic],
        pit2,
        clock,
    }
}

#[cfg(target_arch = "x86_64")]
fn mmio(slot: u32) -> MmioDeviceSnapshot {
    MmioDeviceSnapshot {
        slot,
        transport: VirtioMmioSnapshot {
            device_type: 26,
            device_state: b"virtiofs-state".to_vec(),
            status: 0xf,
            features_sel: 1,
            driver_features: 0x1000_0000,
            driver_features_sel: 0,
            queue_sel: 1,
            queues: vec![QueueSnapshot {
                num: 16,
                ready: true,
                desc_lo: 0x1000,
                desc_hi: 0,
                driver_lo: 0x2000,
                driver_hi: 0,
                device_lo: 0x3000,
                device_hi: 0,
            }],
            interrupt_status: 1,
            config_generation: 2,
            activated: true,
        },
    }
}

#[cfg(target_arch = "x86_64")]
#[test]
fn rejects_oversized_mmio_device_state_before_allocation() {
    let mut encoded = Vec::new();
    write_u32(&mut encoded, 4).unwrap();
    write_u32(&mut encoded, 26).unwrap();
    write_u32(&mut encoded, (MAX_DEVICE_STATE_BYTES + 1) as u32).unwrap();

    let err = read_mmio_device_snapshot(&mut encoded.as_slice()).unwrap_err();

    assert!(err.to_string().contains("device state length"), "{err:#}");
}

#[cfg(target_arch = "x86_64")]
fn encoded_mmio(snapshot: &MmioDeviceSnapshot) -> Vec<u8> {
    let mut encoded = Vec::new();
    write_mmio_device_snapshot(&mut encoded, snapshot).unwrap();
    encoded
}

#[cfg(target_arch = "x86_64")]
#[test]
fn rejects_total_mmio_device_state_before_allocation() {
    let snapshot = mmio(4);
    let encoded = encoded_mmio(&snapshot);
    let budget = snapshot.transport.device_state.len() - 1;

    let err = read_mmio_device_snapshot_with_budget(&mut encoded.as_slice(), budget).unwrap_err();

    assert!(err.to_string().contains("total device state"), "{err:#}");
}

#[cfg(target_arch = "x86_64")]
#[test]
fn rejects_non_boolean_mmio_activation_and_queue_ready_bytes() {
    let snapshot = mmio(4);
    let encoded = encoded_mmio(&snapshot);
    let activated_offset = 12 + snapshot.transport.device_state.len() + 32;
    let queue_ready_offset = activated_offset + 1 + 4 + 2;

    let mut bad_activation = encoded.clone();
    bad_activation[activated_offset] = 2;
    let err = read_mmio_device_snapshot(&mut bad_activation.as_slice()).unwrap_err();
    assert!(err.to_string().contains("MMIO activated"), "{err:#}");

    let mut bad_queue = encoded;
    bad_queue[queue_ready_offset] = 2;
    let err = read_mmio_device_snapshot(&mut bad_queue.as_slice()).unwrap_err();
    assert!(err.to_string().contains("MMIO queue ready"), "{err:#}");
}

#[cfg(target_arch = "x86_64")]
#[test]
fn rejects_oversized_mmio_queue_count_before_allocation() {
    let snapshot = mmio(4);
    let mut encoded = encoded_mmio(&snapshot);
    let activated_offset = 12 + snapshot.transport.device_state.len() + 32;
    let queue_count_offset = activated_offset + 1;
    encoded[queue_count_offset..queue_count_offset + 4]
        .copy_from_slice(&((MAX_QUEUES_PER_DEVICE + 1) as u32).to_le_bytes());
    encoded.truncate(queue_count_offset + 4);

    let err = read_mmio_device_snapshot(&mut encoded.as_slice()).unwrap_err();

    assert!(
        err.to_string().contains("queue count exceeds limit"),
        "{err:#}"
    );
}

#[cfg(target_arch = "x86_64")]
#[test]
fn writes_header_and_memory() {
    let dir = temp_dir("writes-header-memory");
    let path = dir.join("state.kvm");
    let mem = GuestMemory::new(8192).unwrap();
    mem.write_at(0, b"hello").unwrap();
    mem.write_at(4096, b"world").unwrap();

    write_checkpoint(
        &path,
        &mem,
        &[snapshot(0), snapshot(1)],
        &vm_snapshot(),
        &[mmio(0)],
    )
    .unwrap();

    let bytes = std::fs::read(path).unwrap();
    let header = CheckpointHeader::decode(&bytes[..HEADER_LEN as usize]).unwrap();
    assert_eq!(header.ram_bytes, 8192);
    let memory_offset = bytes.len() - 8192;
    assert_eq!(&bytes[memory_offset..memory_offset + 5], b"hello");
    assert_eq!(&bytes[memory_offset + 4096..memory_offset + 4101], b"world");
    assert_eq!(bytes.len(), memory_offset + 8192);
}

#[cfg(target_arch = "x86_64")]
#[test]
fn restores_memory_and_vcpu_state() {
    let dir = temp_dir("restore-memory-vcpu");
    let path = dir.join("state.kvm");
    let mem = GuestMemory::new(8192).unwrap();
    mem.write_at(0, b"hello").unwrap();
    mem.write_at(4096, b"world").unwrap();
    write_checkpoint(
        &path,
        &mem,
        &[snapshot(0), snapshot(1)],
        &vm_snapshot(),
        &[mmio(3)],
    )
    .unwrap();

    let restored_mem = GuestMemory::new(8192).unwrap();
    let restored = read_checkpoint(&path, &restored_mem, 2, 1).unwrap();

    let mut buf = [0u8; 5];
    restored_mem.read_at(0, &mut buf).unwrap();
    assert_eq!(&buf, b"hello");
    restored_mem.read_at(4096, &mut buf).unwrap();
    assert_eq!(&buf, b"world");
    assert_eq!(restored.vcpus.len(), 2);
    assert_eq!(restored.vcpus[1].regs.rip, 0x1001);
    assert_eq!(restored.vcpus[1].sregs.cr3, 0x2001);
    assert_eq!(restored.vcpus[1].mp_state.mp_state, KVM_MP_STATE_RUNNABLE);
    assert_eq!(restored.vcpus[1].msrs[0].index, 0x6e0);
    assert_eq!(restored.vcpus[1].msrs[0].data, 0x1001);
    assert_eq!(restored.vm, vm_snapshot());
    assert_eq!(restored.mmio_devices, vec![mmio(3)]);
}

#[cfg(all(target_arch = "x86_64", unix))]
#[test]
fn zero_memory_is_written_as_sparse_holes() {
    use std::os::unix::fs::MetadataExt;

    let dir = temp_dir("sparse-zero-memory");
    let path = dir.join("state.kvm");
    let mem_size = 64 * 1024 * 1024;
    let mem = GuestMemory::new(mem_size).unwrap();
    mem.write_at(0, b"front").unwrap();
    mem.write_at(mem_size - 4, b"tail").unwrap();

    write_checkpoint(&path, &mem, &[snapshot(0)], &vm_snapshot(), &[]).unwrap();

    let metadata = std::fs::metadata(&path).unwrap();
    let logical_len = HEADER_LEN
        + 4
        + X86_VCPU_STATE_LEN as u64
        + (3 * std::mem::size_of::<KvmIrqchip>()) as u64
        + std::mem::size_of::<KvmPitState2>() as u64
        + std::mem::size_of::<KvmClockData>() as u64
        + mem_size;
    assert_eq!(metadata.len(), logical_len);
    assert!(
        metadata.blocks() * 512 < logical_len / 2,
        "zero memory should be sparse: allocated={} logical={logical_len}",
        metadata.blocks() * 512
    );

    let restored_mem = GuestMemory::new(mem_size).unwrap();
    read_checkpoint(&path, &restored_mem, 1, 0).unwrap();
    let mut front = [0u8; 5];
    let mut middle = [1u8; 16];
    let mut tail = [0u8; 4];
    restored_mem.read_at(0, &mut front).unwrap();
    restored_mem.read_at(mem_size / 2, &mut middle).unwrap();
    restored_mem.read_at(mem_size - 4, &mut tail).unwrap();
    assert_eq!(&front, b"front");
    assert_eq!(middle, [0u8; 16]);
    assert_eq!(&tail, b"tail");
}

#[cfg(target_arch = "x86_64")]
#[test]
fn overwrites_atomically() {
    let dir = temp_dir("atomic-overwrite");
    let path = dir.join("state.kvm");
    std::fs::write(&path, b"old").unwrap();
    let mem = GuestMemory::new(4096).unwrap();

    write_checkpoint(&path, &mem, &[snapshot(0)], &vm_snapshot(), &[]).unwrap();

    let bytes = std::fs::read(path).unwrap();
    assert_ne!(&bytes, b"old");
    assert_eq!(
        bytes.len(),
        HEADER_LEN as usize
            + 4
            + X86_VCPU_STATE_LEN as usize
            + (3 * std::mem::size_of::<KvmIrqchip>())
            + std::mem::size_of::<KvmPitState2>()
            + std::mem::size_of::<KvmClockData>()
            + 4096
    );
    assert!(std::fs::read_dir(&dir).unwrap().all(|e| !e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .contains(".tmp.")));
}

#[cfg(target_arch = "x86_64")]
#[test]
fn rejects_missing_parent() {
    let dir = temp_dir("missing-parent");
    let path = dir.join("missing").join("state.kvm");
    let mem = GuestMemory::new(4096).unwrap();

    let err = write_checkpoint(&path, &mem, &[snapshot(0)], &vm_snapshot(), &[]).unwrap_err();

    assert!(err
        .to_string()
        .contains("checkpoint parent directory does not exist"));
}

#[cfg(target_arch = "x86_64")]
#[test]
fn removes_temp_file_after_create_failure() {
    let dir = temp_dir("temp-cleanup");
    let path = dir.join("state.kvm");
    let tmp = temp_path_for(&path);
    std::fs::write(&tmp, b"conflict").unwrap();
    let mem = GuestMemory::new(4096).unwrap();

    let err = write_checkpoint(&path, &mem, &[snapshot(0)], &vm_snapshot(), &[]).unwrap_err();

    assert!(err.to_string().contains("create checkpoint temp file"));
    assert!(!tmp.exists());
    assert!(!path.exists());
}

#[cfg(target_arch = "x86_64")]
#[test]
fn restore_rejects_wrong_ram_size() {
    let dir = temp_dir("wrong-ram-size");
    let path = dir.join("state.kvm");
    let mem = GuestMemory::new(4096).unwrap();
    write_checkpoint(&path, &mem, &[snapshot(0)], &vm_snapshot(), &[]).unwrap();
    let larger_mem = GuestMemory::new(8192).unwrap();

    let err = read_checkpoint(&path, &larger_mem, 1, 0).unwrap_err();

    assert!(err.to_string().contains("checkpoint RAM size mismatch"));
}

#[cfg(target_arch = "x86_64")]
#[test]
fn restore_rejects_wrong_vcpu_count() {
    let dir = temp_dir("wrong-vcpu-count");
    let path = dir.join("state.kvm");
    let mem = GuestMemory::new(4096).unwrap();
    write_checkpoint(&path, &mem, &[snapshot(0)], &vm_snapshot(), &[]).unwrap();

    let err = read_checkpoint(&path, &mem, 2, 0).unwrap_err();

    assert!(err.to_string().contains("checkpoint vCPU count mismatch"));
}

#[cfg(target_arch = "x86_64")]
#[test]
fn restore_rejects_trailing_bytes() {
    let dir = temp_dir("trailing-bytes");
    let path = dir.join("state.kvm");
    let mem = GuestMemory::new(4096).unwrap();
    write_checkpoint(&path, &mem, &[snapshot(0)], &vm_snapshot(), &[]).unwrap();
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"extra")
        .unwrap();

    let err = read_checkpoint(&path, &mem, 1, 0).unwrap_err();

    assert!(err.to_string().contains("checkpoint has trailing bytes"));
}
