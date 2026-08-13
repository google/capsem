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
        _vcpu_handles: std::sync::Mutex::new(Vec::new()),
        _vsock_listener_handles: std::sync::Mutex::new(Vec::new()),
        _vsock_irq_handles: std::sync::Mutex::new(Vec::new()),
        #[cfg(target_arch = "x86_64")]
        _mmio_transports: Vec::new(),
        _mmio_bus: Arc::new(mmio::MmioBus::new()),
        _vm: None,
        _guest_mem: memory::GuestMemory::new(4096).unwrap(),
    }
}

#[cfg(target_arch = "x86_64")]
fn snapshot(id: u32) -> checkpoint::VcpuSnapshot {
    let regs = sys::KvmRegs {
        rip: 0x1000 + u64::from(id),
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
fn kvm_stop_joins_vcpu_workers_before_returning() {
    let control = Arc::new(vcpu::VcpuControl::new(0));
    let worker_exited = Arc::new(AtomicBool::new(false));
    let worker = {
        let control = Arc::clone(&control);
        let worker_exited = Arc::clone(&worker_exited);
        std::thread::spawn(move || {
            while !control.is_stopped() {
                std::thread::yield_now();
            }
            std::thread::sleep(Duration::from_millis(50));
            worker_exited.store(true, Ordering::SeqCst);
            Ok(())
        })
    };
    let mut handle = test_handle_with_control(control);
    handle._vcpu_handles.get_mut().unwrap().push(worker);

    handle.stop().unwrap();

    assert!(
        worker_exited.load(Ordering::SeqCst),
        "stop returned while a vCPU worker could still access VM-owned memory"
    );
}

#[test]
fn kvm_drop_joins_vsock_workers_before_releasing_resources() {
    let worker_exited = Arc::new(AtomicBool::new(false));
    let mut handle = test_handle();
    let shutdown = Arc::clone(&handle.shutdown);
    let worker = {
        let worker_exited = Arc::clone(&worker_exited);
        std::thread::spawn(move || {
            while !shutdown.load(Ordering::SeqCst) {
                std::thread::yield_now();
            }
            std::thread::sleep(Duration::from_millis(50));
            worker_exited.store(true, Ordering::SeqCst);
        })
    };
    handle
        ._vsock_listener_handles
        .get_mut()
        .unwrap()
        .push(worker);

    drop(handle);

    assert!(
        worker_exited.load(Ordering::SeqCst),
        "drop released VM-owned resources before a vsock worker exited"
    );
}

#[test]
fn kvm_state_decoder_preserves_transient_states() {
    assert_eq!(state_from_u8(VmState::Pausing as u8), VmState::Pausing);
    assert_eq!(state_from_u8(VmState::Resuming as u8), VmState::Resuming);
    assert_eq!(state_from_u8(VmState::Saving as u8), VmState::Saving);
    assert_eq!(state_from_u8(255), VmState::Unknown);
}

#[cfg(target_arch = "x86_64")]
fn mmio_slot(slot: u32, device_type: u32) -> checkpoint::MmioDeviceSnapshot {
    checkpoint::MmioDeviceSnapshot {
        slot,
        transport: virtio_mmio::VirtioMmioSnapshot {
            device_type,
            device_state: Vec::new(),
            status: 0,
            features_sel: 0,
            driver_features: 0,
            driver_features_sel: 0,
            queue_sel: 0,
            queues: vec![virtio_mmio::QueueSnapshot::default()],
            interrupt_status: 0,
            config_generation: 0,
            activated: false,
        },
    }
}

#[cfg(target_arch = "x86_64")]
#[test]
fn restore_rejects_duplicate_mmio_slots_before_device_activation() {
    let err = validate_mmio_topology(&[(0, 3), (4, 26)], &[mmio_slot(4, 26), mmio_slot(4, 26)])
        .unwrap_err();

    assert!(err.to_string().contains("duplicate MMIO slot"), "{err:#}");
}

#[cfg(target_arch = "x86_64")]
#[test]
fn restore_rejects_missing_mmio_slot_before_device_activation() {
    let err = validate_mmio_topology(&[(0, 3), (4, 26)], &[mmio_slot(0, 3)]).unwrap_err();

    assert!(err.to_string().contains("topology mismatch"), "{err:#}");
}

#[cfg(target_arch = "x86_64")]
#[test]
fn restore_rejects_mmio_device_type_mismatch_before_preparation() {
    let err = validate_mmio_topology(&[(0, 3), (4, 26)], &[mmio_slot(0, 3), mmio_slot(4, 19)])
        .unwrap_err();

    assert!(err.to_string().contains("topology mismatch"), "{err:#}");
    assert!(err.to_string().contains("device type"), "{err:#}");
}

#[cfg(target_arch = "x86_64")]
struct RestoreOrderDevice {
    device_type: u32,
    label: &'static str,
    reject_state: bool,
    events: Arc<std::sync::Mutex<Vec<&'static str>>>,
    activated: Arc<AtomicBool>,
}

#[cfg(target_arch = "x86_64")]
impl virtio_mmio::VirtioDevice for RestoreOrderDevice {
    fn device_type(&self) -> u32 {
        self.device_type
    }

    fn features(&self) -> u64 {
        1 << 32
    }

    fn queue_max_sizes(&self) -> &[u16] {
        &[8]
    }

    fn read_config(&self, _offset: u64, _data: &mut [u8]) {}

    fn write_config(&self, _offset: u64, _data: &[u8]) {}

    fn activate(&mut self, _mem: memory::GuestMemoryRef, _queues: &[virtio_mmio::QueueConfig]) {
        self.activated.store(true, Ordering::SeqCst);
    }

    fn restore_checkpoint_state(&mut self, _state: &[u8]) -> Result<()> {
        self.events.lock().unwrap().push(self.label);
        ensure!(!self.reject_state, "rejected device identity");
        Ok(())
    }

    fn restore_activate(
        &mut self,
        _mem: memory::GuestMemoryRef,
        _queues: &[virtio_mmio::QueueConfig],
    ) -> Result<()> {
        let events = self.events.lock().unwrap();
        ensure!(
            events.len() >= 2,
            "device activated before the complete graph was prepared"
        );
        drop(events);
        self.activated.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn queue_notify(&mut self, _queue_index: u32) -> bool {
        false
    }
}

#[cfg(target_arch = "x86_64")]
fn active_mmio_slot(slot: u32, device_type: u32, ring_base: u32) -> checkpoint::MmioDeviceSnapshot {
    let mut snapshot = mmio_slot(slot, device_type);
    snapshot.transport.status = 0xf;
    snapshot.transport.driver_features = 1 << 32;
    snapshot.transport.queues[0] = virtio_mmio::QueueSnapshot {
        num: 8,
        ready: true,
        desc_lo: ring_base,
        desc_hi: 0,
        driver_lo: ring_base + 0x200,
        driver_hi: 0,
        device_lo: ring_base + 0x400,
        device_hi: 0,
    };
    snapshot.transport.activated = true;
    snapshot
}

#[cfg(target_arch = "x86_64")]
#[test]
fn restore_prepares_complete_mmio_graph_before_any_activation() {
    let memory = memory::GuestMemory::new(4096).unwrap();
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let first_activated = Arc::new(AtomicBool::new(false));
    let second_activated = Arc::new(AtomicBool::new(false));
    let transports = vec![
        (
            0,
            Arc::new(virtio_mmio::VirtioMmioTransport::new(
                Box::new(RestoreOrderDevice {
                    device_type: 40,
                    label: "prepare-first",
                    reject_state: false,
                    events: Arc::clone(&events),
                    activated: Arc::clone(&first_activated),
                }),
                memory.clone_ref(memory::RAM_BASE),
            )),
        ),
        (
            4,
            Arc::new(virtio_mmio::VirtioMmioTransport::new(
                Box::new(RestoreOrderDevice {
                    device_type: 41,
                    label: "prepare-second",
                    reject_state: false,
                    events: Arc::clone(&events),
                    activated: Arc::clone(&second_activated),
                }),
                memory.clone_ref(memory::RAM_BASE),
            )),
        ),
    ];

    restore_mmio_device_graph(
        &transports,
        &[
            active_mmio_slot(0, 40, 0x100),
            active_mmio_slot(4, 41, 0x900),
        ],
    )
    .unwrap();

    assert_eq!(
        *events.lock().unwrap(),
        vec!["prepare-first", "prepare-second"]
    );
    assert!(first_activated.load(Ordering::SeqCst));
    assert!(second_activated.load(Ordering::SeqCst));
}

#[cfg(target_arch = "x86_64")]
#[test]
fn restore_rejects_queue_memory_overlap_across_mmio_slots_before_preparation() {
    let memory = memory::GuestMemory::new(4096).unwrap();
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let first_activated = Arc::new(AtomicBool::new(false));
    let second_activated = Arc::new(AtomicBool::new(false));
    let transports = vec![
        (
            0,
            Arc::new(virtio_mmio::VirtioMmioTransport::new(
                Box::new(RestoreOrderDevice {
                    device_type: 40,
                    label: "prepare-first",
                    reject_state: false,
                    events: Arc::clone(&events),
                    activated: Arc::clone(&first_activated),
                }),
                memory.clone_ref(memory::RAM_BASE),
            )),
        ),
        (
            4,
            Arc::new(virtio_mmio::VirtioMmioTransport::new(
                Box::new(RestoreOrderDevice {
                    device_type: 41,
                    label: "prepare-second",
                    reject_state: false,
                    events: Arc::clone(&events),
                    activated: Arc::clone(&second_activated),
                }),
                memory.clone_ref(memory::RAM_BASE),
            )),
        ),
    ];

    let err = restore_mmio_device_graph(
        &transports,
        &[
            active_mmio_slot(0, 40, 0x100),
            active_mmio_slot(4, 41, 0x100),
        ],
    )
    .unwrap_err();

    assert!(err.to_string().contains("overlap"), "{err:#}");
    assert!(events.lock().unwrap().is_empty());
    assert!(!first_activated.load(Ordering::SeqCst));
    assert!(!second_activated.load(Ordering::SeqCst));
}

#[cfg(target_arch = "x86_64")]
#[test]
fn restore_preparation_failure_leaves_earlier_device_inactive() {
    let memory = memory::GuestMemory::new(4096).unwrap();
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let first_activated = Arc::new(AtomicBool::new(false));
    let second_activated = Arc::new(AtomicBool::new(false));
    let transports = vec![
        (
            0,
            Arc::new(virtio_mmio::VirtioMmioTransport::new(
                Box::new(RestoreOrderDevice {
                    device_type: 40,
                    label: "prepare-first",
                    reject_state: false,
                    events: Arc::clone(&events),
                    activated: Arc::clone(&first_activated),
                }),
                memory.clone_ref(memory::RAM_BASE),
            )),
        ),
        (
            4,
            Arc::new(virtio_mmio::VirtioMmioTransport::new(
                Box::new(RestoreOrderDevice {
                    device_type: 41,
                    label: "prepare-second",
                    reject_state: true,
                    events: Arc::clone(&events),
                    activated: Arc::clone(&second_activated),
                }),
                memory.clone_ref(memory::RAM_BASE),
            )),
        ),
    ];

    let err = restore_mmio_device_graph(
        &transports,
        &[
            active_mmio_slot(0, 40, 0x100),
            active_mmio_slot(4, 41, 0x900),
        ],
    )
    .unwrap_err();

    assert!(
        err.to_string().contains("rejected device identity"),
        "{err:#}"
    );
    assert!(!first_activated.load(Ordering::SeqCst));
    assert!(!second_activated.load(Ordering::SeqCst));
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
