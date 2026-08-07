use super::super::mmio::MmioDevice;
use super::*;
use std::sync::atomic::AtomicU32;

struct CountingDevice {
    reads: AtomicU32,
    writes: AtomicU32,
}

impl CountingDevice {
    fn new() -> Self {
        Self {
            reads: AtomicU32::new(0),
            writes: AtomicU32::new(0),
        }
    }
}

impl MmioDevice for CountingDevice {
    fn read(&self, _offset: u64, data: &mut [u8]) {
        self.reads.fetch_add(1, Ordering::SeqCst);
        data.fill(0);
    }

    fn write(&self, _offset: u64, _data: &[u8]) {
        self.writes.fetch_add(1, Ordering::SeqCst);
    }
}

#[cfg(target_arch = "x86_64")]
fn snapshot(id: u32) -> checkpoint::VcpuSnapshot {
    checkpoint::VcpuSnapshot {
        id,
        regs: super::super::sys::KvmRegs::default(),
        sregs: super::super::sys::KvmSregs::default(),
        mp_state: super::super::sys::KvmMpState {
            mp_state: super::super::sys::KVM_MP_STATE_RUNNABLE,
        },
        msrs: Vec::new(),
        lapic: super::super::sys::KvmLapicState::default(),
        events: super::super::sys::KvmVcpuEvents::default(),
        debugregs: super::super::sys::KvmDebugRegs::default(),
        fpu: super::super::sys::KvmFpu::default(),
        xcrs: super::super::sys::KvmXcrs::default(),
        xsave: super::super::sys::KvmXsave::default(),
    }
}

#[test]
fn mmio_bus_wired_to_device() {
    // Verify the MMIO bus can be shared across threads (simulating vCPU access)
    let bus = Arc::new(MmioBus::new());
    let dev = Arc::new(CountingDevice::new());
    bus.register(0x1000, 0x100, dev.clone()).unwrap();

    let bus_clone = Arc::clone(&bus);
    let handle = std::thread::spawn(move || {
        let mut data = [0u8; 4];
        bus_clone.read(0x1000, &mut data);
        bus_clone.write(0x1050, &[1, 2, 3, 4]);
    });
    handle.join().unwrap();

    assert_eq!(dev.reads.load(Ordering::SeqCst), 1);
    assert_eq!(dev.writes.load(Ordering::SeqCst), 1);
}

#[test]
fn shutdown_flag_stops_loop_concept() {
    // We can't test the actual KVM_RUN loop without /dev/kvm,
    // but we can verify the shutdown flag mechanics
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown2 = Arc::clone(&shutdown);

    let handle = std::thread::spawn(move || {
        // Simulate checking shutdown in a loop
        let mut iters = 0;
        loop {
            if shutdown2.load(Ordering::Relaxed) {
                return iters;
            }
            iters += 1;
            std::thread::yield_now();
            if iters > 10000 {
                return iters; // safety valve
            }
        }
    });

    // Let the thread spin a bit, then signal shutdown
    std::thread::sleep(std::time::Duration::from_millis(1));
    shutdown.store(true, Ordering::SeqCst);

    let iters = handle.join().unwrap();
    assert!(
        iters < 10000,
        "thread should have stopped, ran {iters} iterations"
    );
}

#[test]
fn pause_waits_for_all_vcpus_to_park() {
    let control = Arc::new(VcpuControl::new(2));
    let mut handles = Vec::new();
    for id in 0..2 {
        let c = Arc::clone(&control);
        handles.push(std::thread::spawn(move || loop {
            if c.is_stopped() {
                break;
            }
            #[cfg(target_arch = "x86_64")]
            c.wait_if_paused(id, || Ok(snapshot(id))).unwrap();
            #[cfg(not(target_arch = "x86_64"))]
            c.wait_if_paused();
            std::thread::yield_now();
        }));
    }

    control.request_pause(Duration::from_secs(1)).unwrap();
    assert_eq!(control.lifecycle.load(Ordering::SeqCst), VCPU_PAUSED);
    control.resume().unwrap();
    assert_eq!(control.lifecycle.load(Ordering::SeqCst), VCPU_RUNNING);
    control.request_stop();
    for handle in handles {
        handle.join().unwrap();
    }
}

#[test]
fn pause_times_out_when_vcpu_does_not_park() {
    let control = VcpuControl::new(1);
    let err = control.request_pause(Duration::from_millis(1)).unwrap_err();

    assert!(err.to_string().contains("timed out pausing KVM VM"));
    assert_eq!(control.lifecycle.load(Ordering::SeqCst), VCPU_RUNNING);
}

#[test]
fn kick_targets_registered_vcpu_threads() {
    let control = VcpuControl::new(1);
    let registration = control.register_current_thread(0).unwrap();

    assert_eq!(control.kick_vcpus(), 1);
    drop(registration);
    assert_eq!(control.kick_vcpus(), 0);
}

#[test]
fn register_rejects_out_of_range_vcpu() {
    let control = VcpuControl::new(1);
    let err = match control.register_current_thread(1) {
        Ok(_) => panic!("out-of-range vCPU registration should fail"),
        Err(err) => err,
    };

    assert!(err.to_string().contains("outside thread table"));
}

#[test]
fn stop_unblocks_paused_vcpus() {
    let control = Arc::new(VcpuControl::new(1));
    let c = Arc::clone(&control);
    let handle = std::thread::spawn(move || loop {
        if c.is_stopped() {
            break true;
        }
        #[cfg(target_arch = "x86_64")]
        c.wait_if_paused(0, || Ok(snapshot(0))).unwrap();
        #[cfg(not(target_arch = "x86_64"))]
        c.wait_if_paused();
        std::thread::yield_now();
    });

    control.request_pause(Duration::from_secs(1)).unwrap();
    control.request_stop();

    assert!(handle.join().unwrap());
    assert_eq!(control.lifecycle.load(Ordering::SeqCst), VCPU_STOPPED);
}

#[test]
fn stopped_vm_cannot_pause_or_resume() {
    let control = VcpuControl::new(0);
    control.request_stop();

    assert!(control
        .request_pause(Duration::from_millis(1))
        .unwrap_err()
        .to_string()
        .contains("cannot pause stopped"));
    assert!(control
        .resume()
        .unwrap_err()
        .to_string()
        .contains("cannot resume stopped"));
}

#[cfg(target_arch = "x86_64")]
#[test]
fn hlt_exit_continues_until_shutdown_requested() {
    assert_eq!(hlt_exit_action(false), HltExitAction::Continue);
    assert_eq!(hlt_exit_action(true), HltExitAction::Stop);
}

#[cfg(target_arch = "x86_64")]
#[test]
fn pause_collects_vcpu_snapshots() {
    let control = Arc::new(VcpuControl::new(1));
    let c = Arc::clone(&control);
    let (initial_poll_tx, initial_poll_rx) = std::sync::mpsc::sync_channel(0);
    let handle = std::thread::spawn(move || {
        c.wait_if_paused(0, || Ok(snapshot(0))).unwrap();
        initial_poll_tx.send(()).unwrap();
        loop {
            if c.is_stopped() {
                break;
            }
            c.wait_if_paused(0, || Ok(snapshot(0))).unwrap();
            std::thread::yield_now();
        }
    });

    // Prove the vCPU observed RUNNING before the pause request. A real vCPU
    // keeps polling; a one-shot test thread can exit here and race the pause.
    initial_poll_rx.recv().unwrap();
    control.request_pause(Duration::from_secs(1)).unwrap();
    let snapshots = control.snapshots().unwrap();

    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].id, 0);
    control.resume().unwrap();
    control.request_stop();
    handle.join().unwrap();
}

#[cfg(target_arch = "x86_64")]
struct CountingPioDevice {
    reads: AtomicU32,
    writes: AtomicU32,
}

#[cfg(target_arch = "x86_64")]
impl CountingPioDevice {
    fn new() -> Self {
        Self {
            reads: AtomicU32::new(0),
            writes: AtomicU32::new(0),
        }
    }
}

#[cfg(target_arch = "x86_64")]
impl super::super::pio::PioDevice for CountingPioDevice {
    fn read(&self, _offset: u16, data: &mut [u8]) {
        self.reads.fetch_add(1, Ordering::SeqCst);
        data.fill(0);
    }

    fn write(&self, _offset: u16, _data: &[u8]) {
        self.writes.fetch_add(1, Ordering::SeqCst);
    }
}

#[cfg(target_arch = "x86_64")]
#[test]
fn dispatch_pio_respects_count() {
    let bus = Arc::new(PioBus::new());
    let dev = Arc::new(CountingPioDevice::new());
    bus.register(0x3F8, 8, dev.clone()).unwrap();

    let mut data = [0u8; 4]; // 4 bytes of data
                             // Simulate string I/O out: 4 bytes written 1 byte at a time
    dispatch_pio(&bus, 1, 0x3F8, 1, 4, data.as_mut_ptr());

    assert_eq!(
        dev.writes.load(Ordering::SeqCst),
        4,
        "PIO dispatch ignored count > 1"
    );
}
