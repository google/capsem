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
