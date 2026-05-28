//! vCPU run loop: executes guest code and dispatches MMIO exits.
//!
//! Each vCPU runs on its own OS thread. The run loop calls KVM_RUN
//! in a tight loop, handling MMIO exits by dispatching to the MMIO bus,
//! pausing when the lifecycle controller requests it, and stopping when the
//! guest or host requests shutdown.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, Once};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use tracing::{debug, info, warn};

#[cfg(target_arch = "x86_64")]
use super::checkpoint;
use super::mmio::MmioBus;
#[cfg(target_arch = "x86_64")]
use super::pio::PioBus;
use super::sys::{VcpuExit, VcpuFd, KVM_SYSTEM_EVENT_RESET, KVM_SYSTEM_EVENT_SHUTDOWN};

const VCPU_RUNNING: u8 = 0;
const VCPU_PAUSING: u8 = 1;
const VCPU_PAUSED: u8 = 2;
const VCPU_STOPPED: u8 = 3;
const VCPU_KICK_SIGNAL: libc::c_int = libc::SIGUSR1;
static INSTALL_KICK_HANDLER: Once = Once::new();

/// Cooperative vCPU lifecycle controller.
///
/// KVM does not provide a portable "pause all vCPUs" ioctl. Capsem parks each
/// vCPU at the top of its run-loop, after KVM_RUN has returned and before the
/// next guest entry. Pause/stop requests also send a targeted signal to each
/// registered vCPU thread so a blocking `KVM_RUN` returns with EINTR promptly.
pub(super) struct VcpuControl {
    state: AtomicBool,
    lifecycle: std::sync::atomic::AtomicU8,
    paused_count: Mutex<u32>,
    threads: Mutex<Vec<Option<libc::pthread_t>>>,
    #[cfg(target_arch = "x86_64")]
    snapshots: Mutex<Vec<Option<checkpoint::VcpuSnapshot>>>,
    pause_cv: Condvar,
    vcpu_count: u32,
}

impl VcpuControl {
    pub fn new(vcpu_count: u32) -> Self {
        Self {
            state: AtomicBool::new(false),
            lifecycle: std::sync::atomic::AtomicU8::new(VCPU_RUNNING),
            paused_count: Mutex::new(0),
            threads: Mutex::new(vec![None; vcpu_count as usize]),
            #[cfg(target_arch = "x86_64")]
            snapshots: Mutex::new(vec![None; vcpu_count as usize]),
            pause_cv: Condvar::new(),
            vcpu_count,
        }
    }

    pub fn request_stop(&self) {
        self.state.store(true, Ordering::SeqCst);
        self.lifecycle.store(VCPU_STOPPED, Ordering::SeqCst);
        self.kick_vcpus();
        self.pause_cv.notify_all();
    }

    pub fn is_stopped(&self) -> bool {
        self.state.load(Ordering::SeqCst) || self.lifecycle.load(Ordering::SeqCst) == VCPU_STOPPED
    }

    pub fn request_pause(&self, timeout: Duration) -> Result<()> {
        match self.lifecycle.compare_exchange(
            VCPU_RUNNING,
            VCPU_PAUSING,
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            Ok(_) => {}
            Err(VCPU_PAUSED) => return Ok(()),
            Err(VCPU_PAUSING) => {}
            Err(VCPU_STOPPED) => bail!("cannot pause stopped KVM VM"),
            Err(other) => bail!("cannot pause KVM VM from lifecycle state {other}"),
        }

        #[cfg(target_arch = "x86_64")]
        {
            self.snapshots
                .lock()
                .expect("snapshot mutex poisoned")
                .fill(None);
        }
        self.pause_cv.notify_all();
        self.kick_vcpus();
        let deadline = Instant::now() + timeout;
        let mut paused = self.paused_count.lock().expect("pause mutex poisoned");
        while *paused < self.vcpu_count {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                self.lifecycle.store(VCPU_RUNNING, Ordering::SeqCst);
                self.pause_cv.notify_all();
                bail!(
                    "timed out pausing KVM VM: {}/{} vCPUs parked",
                    *paused,
                    self.vcpu_count
                );
            };
            let (guard, wait) = self
                .pause_cv
                .wait_timeout(paused, remaining)
                .expect("pause condvar poisoned");
            paused = guard;
            if wait.timed_out() && *paused < self.vcpu_count {
                self.lifecycle.store(VCPU_RUNNING, Ordering::SeqCst);
                self.pause_cv.notify_all();
                bail!(
                    "timed out pausing KVM VM: {}/{} vCPUs parked",
                    *paused,
                    self.vcpu_count
                );
            }
        }
        self.lifecycle.store(VCPU_PAUSED, Ordering::SeqCst);
        self.pause_cv.notify_all();
        Ok(())
    }

    pub fn resume(&self) -> Result<()> {
        match self.lifecycle.load(Ordering::SeqCst) {
            VCPU_RUNNING => Ok(()),
            VCPU_PAUSING | VCPU_PAUSED => {
                self.lifecycle.store(VCPU_RUNNING, Ordering::SeqCst);
                self.pause_cv.notify_all();
                Ok(())
            }
            VCPU_STOPPED => bail!("cannot resume stopped KVM VM"),
            other => bail!("cannot resume KVM VM from lifecycle state {other}"),
        }
    }

    pub fn register_current_thread(&self, vcpu_id: u32) -> Result<VcpuThreadRegistration<'_>> {
        install_kick_handler();
        let mut threads = self.threads.lock().expect("thread mutex poisoned");
        let slot = threads
            .get_mut(vcpu_id as usize)
            .ok_or_else(|| anyhow::anyhow!("vCPU id {vcpu_id} outside thread table"))?;
        *slot = Some(unsafe { libc::pthread_self() });
        Ok(VcpuThreadRegistration {
            control: self,
            vcpu_id,
        })
    }

    fn unregister_thread(&self, vcpu_id: u32) {
        if let Some(slot) = self
            .threads
            .lock()
            .expect("thread mutex poisoned")
            .get_mut(vcpu_id as usize)
        {
            *slot = None;
        }
    }

    fn kick_vcpus(&self) -> usize {
        let threads = self.threads.lock().expect("thread mutex poisoned");
        let mut kicked = 0;
        for thread in threads.iter().flatten() {
            let ret = unsafe { libc::pthread_kill(*thread, VCPU_KICK_SIGNAL) };
            if ret == 0 {
                kicked += 1;
            } else {
                debug!(errno = ret, "failed to kick KVM vCPU thread");
            }
        }
        kicked
    }

    #[cfg(target_arch = "x86_64")]
    pub fn snapshots(&self) -> Result<Vec<checkpoint::VcpuSnapshot>> {
        let snapshots = self.snapshots.lock().expect("snapshot mutex poisoned");
        snapshots
            .iter()
            .enumerate()
            .map(|(idx, snapshot)| {
                snapshot.ok_or_else(|| anyhow::anyhow!("missing KVM vCPU snapshot for vCPU {idx}"))
            })
            .collect()
    }

    #[cfg(target_arch = "x86_64")]
    pub(super) fn wait_if_paused(
        &self,
        vcpu_id: u32,
        snapshot: impl FnOnce() -> Result<checkpoint::VcpuSnapshot>,
    ) -> Result<()> {
        let lifecycle = self.lifecycle.load(Ordering::SeqCst);
        if lifecycle != VCPU_PAUSING && lifecycle != VCPU_PAUSED {
            return Ok(());
        }

        let snapshot = snapshot()?;
        if snapshot.id != vcpu_id {
            bail!(
                "snapshot vCPU id mismatch: snapshot={}, vcpu={}",
                snapshot.id,
                vcpu_id
            );
        }
        {
            let mut snapshots = self.snapshots.lock().expect("snapshot mutex poisoned");
            let slot = snapshots
                .get_mut(vcpu_id as usize)
                .ok_or_else(|| anyhow::anyhow!("vCPU id {vcpu_id} outside snapshot table"))?;
            *slot = Some(snapshot);
        }
        self.wait_parked();
        Ok(())
    }

    #[cfg(not(target_arch = "x86_64"))]
    fn wait_if_paused(&self) {
        let lifecycle = self.lifecycle.load(Ordering::SeqCst);
        if lifecycle != VCPU_PAUSING && lifecycle != VCPU_PAUSED {
            return;
        }
        self.wait_parked();
    }

    fn wait_parked(&self) {
        let mut paused = self.paused_count.lock().expect("pause mutex poisoned");
        *paused += 1;
        self.pause_cv.notify_all();
        while matches!(
            self.lifecycle.load(Ordering::SeqCst),
            VCPU_PAUSING | VCPU_PAUSED
        ) && !self.is_stopped()
        {
            paused = self.pause_cv.wait(paused).expect("pause condvar poisoned");
        }
        *paused = paused.saturating_sub(1);
        self.pause_cv.notify_all();
    }
}

pub(super) struct VcpuThreadRegistration<'a> {
    control: &'a VcpuControl,
    vcpu_id: u32,
}

impl Drop for VcpuThreadRegistration<'_> {
    fn drop(&mut self) {
        self.control.unregister_thread(self.vcpu_id);
    }
}

extern "C" fn vcpu_kick_handler(_: libc::c_int) {}

fn install_kick_handler() {
    INSTALL_KICK_HANDLER.call_once(|| {
        let mut action = unsafe { std::mem::zeroed::<libc::sigaction>() };
        action.sa_sigaction = vcpu_kick_handler as *const () as usize;
        action.sa_flags = 0;
        unsafe {
            libc::sigemptyset(&mut action.sa_mask);
            libc::sigaction(VCPU_KICK_SIGNAL, &action, std::ptr::null_mut());
        }
    });
}

/// Spawn a vCPU run loop thread.
///
/// The thread runs KVM_RUN in a loop, dispatching MMIO exits to the bus.
/// It terminates when:
/// - host lifecycle stop is requested
/// - Guest triggers a system event (PSCI shutdown/reset)
/// - An unrecoverable KVM error occurs
pub(super) fn run_vcpu(
    vcpu: VcpuFd,
    mmio_bus: Arc<MmioBus>,
    #[cfg(target_arch = "x86_64")] pio_bus: Arc<PioBus>,
    control: Arc<VcpuControl>,
) -> JoinHandle<Result<()>> {
    let vcpu_id = vcpu.id();

    std::thread::Builder::new()
        .name(format!("kvm-vcpu-{vcpu_id}"))
        .spawn(move || {
            info!(vcpu_id, "vCPU thread started");
            let registration = control.register_current_thread(vcpu_id)?;
            let result = vcpu_loop(
                &vcpu,
                &mmio_bus,
                #[cfg(target_arch = "x86_64")]
                &pio_bus,
                &control,
            );
            if let Err(error) = &result {
                warn!(vcpu_id, error = %error, "vCPU thread failed");
            }
            drop(registration);
            info!(vcpu_id, "vCPU thread exiting");
            result
        })
        .expect("failed to spawn vCPU thread")
}

fn vcpu_loop(
    vcpu: &VcpuFd,
    mmio_bus: &MmioBus,
    #[cfg(target_arch = "x86_64")] pio_bus: &PioBus,
    control: &VcpuControl,
) -> Result<()> {
    loop {
        if control.is_stopped() {
            #[cfg(target_arch = "x86_64")]
            log_vcpu_shutdown_snapshot(vcpu, "pre_run");
            debug!("vCPU {} shutdown requested", vcpu.id());
            return Ok(());
        }
        #[cfg(target_arch = "x86_64")]
        control.wait_if_paused(vcpu.id(), || checkpoint::snapshot_vcpu(vcpu))?;
        #[cfg(not(target_arch = "x86_64"))]
        control.wait_if_paused();
        if control.is_stopped() {
            debug!("vCPU {} shutdown requested while paused", vcpu.id());
            return Ok(());
        }

        let exit = vcpu.run()?;

        match exit {
            VcpuExit::Mmio {
                addr,
                data_offset: _,
                len,
                is_write,
            } => {
                if is_write {
                    // Read data from kvm_run's MMIO data buffer
                    let data = &vcpu.mmio_data_mut()[..len as usize];
                    mmio_bus.write(addr, data);
                } else {
                    // Read from device into kvm_run's MMIO data buffer
                    let data = &mut vcpu.mmio_data_mut()[..len as usize];
                    mmio_bus.read(addr, data);
                }
            }

            #[cfg(target_arch = "x86_64")]
            VcpuExit::Io {
                direction,
                port,
                size,
            } => {
                let io = vcpu.io_data();
                dispatch_pio(
                    pio_bus,
                    direction,
                    port,
                    size,
                    io.count,
                    vcpu.io_data_mut(io.data_offset),
                );
            }

            #[cfg(target_arch = "x86_64")]
            VcpuExit::Hlt => {
                if hlt_exit_action(control.is_stopped()) == HltExitAction::Stop {
                    info!("guest halted (HLT) after shutdown on vCPU {}", vcpu.id());
                    return Ok(());
                }
                debug!("guest HLT on vCPU {}, re-entering KVM_RUN", vcpu.id());
            }

            #[cfg(target_arch = "x86_64")]
            VcpuExit::Shutdown => {
                warn!("guest triple-fault (shutdown) on vCPU {}", vcpu.id());
                control.request_stop();
                return Ok(());
            }

            #[cfg(target_arch = "x86_64")]
            VcpuExit::FailEntry {
                hardware_entry_failure_reason,
            } => {
                warn!(
                    vcpu_id = vcpu.id(),
                    hardware_entry_failure_reason =
                        format_args!("{hardware_entry_failure_reason:#x}"),
                    "KVM failed guest entry"
                );
                std::thread::sleep(Duration::from_millis(10));
            }

            VcpuExit::SystemEvent { event_type } => match event_type {
                KVM_SYSTEM_EVENT_SHUTDOWN => {
                    info!("guest requested shutdown (PSCI SYSTEM_OFF)");
                    control.request_stop();
                    return Ok(());
                }
                KVM_SYSTEM_EVENT_RESET => {
                    info!("guest requested reset (PSCI SYSTEM_RESET)");
                    control.request_stop();
                    return Ok(());
                }
                other => {
                    warn!("unknown system event type: {other}");
                }
            },

            VcpuExit::Interrupted => {
                // Interrupted by a signal -- check shutdown and retry
                #[cfg(target_arch = "x86_64")]
                if control.is_stopped() {
                    log_vcpu_shutdown_snapshot(vcpu, "interrupted");
                }
                continue;
            }

            VcpuExit::NotReady => {
                // x86 APs return EAGAIN while parked in KVM_MP_STATE_UNINITIALIZED.
                // Linux will make them runnable later via INIT/SIPI.
                std::thread::sleep(Duration::from_millis(1));
            }

            VcpuExit::InternalError => {
                anyhow::bail!("KVM internal error on vCPU {}", vcpu.id());
            }

            VcpuExit::Unknown(reason) => {
                warn!(vcpu_id = vcpu.id(), reason, "unexpected KVM exit");
            }
        }
    }
}

#[cfg(target_arch = "x86_64")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HltExitAction {
    Continue,
    Stop,
}

#[cfg(target_arch = "x86_64")]
fn hlt_exit_action(stop_requested: bool) -> HltExitAction {
    if stop_requested {
        HltExitAction::Stop
    } else {
        HltExitAction::Continue
    }
}

#[cfg(target_arch = "x86_64")]
fn log_vcpu_shutdown_snapshot(vcpu: &VcpuFd, reason: &'static str) {
    match vcpu.get_regs() {
        Ok(regs) => warn!(
            event_name = "kvm.vcpu.shutdown_snapshot",
            vcpu_id = vcpu.id(),
            reason,
            rip = format_args!("{:#x}", regs.rip),
            rsp = format_args!("{:#x}", regs.rsp),
            rflags = format_args!("{:#x}", regs.rflags),
            "KVM vCPU shutdown register snapshot"
        ),
        Err(e) => warn!(
            event_name = "kvm.vcpu.shutdown_snapshot_failed",
            vcpu_id = vcpu.id(),
            reason,
            error = %e,
            "failed to read KVM vCPU register snapshot"
        ),
    }
}

#[cfg(target_arch = "x86_64")]
fn dispatch_pio(
    pio_bus: &PioBus,
    direction: u8,
    port: u16,
    size: u8,
    count: u32,
    data_ptr: *mut u8,
) {
    let size_usize = size as usize;
    if direction == 0 {
        // KVM_EXIT_IO_IN
        for i in 0..count as usize {
            let offset = i * size_usize;
            let data = unsafe { std::slice::from_raw_parts_mut(data_ptr.add(offset), size_usize) };
            pio_bus.read(port, data);
        }
    } else {
        // KVM_EXIT_IO_OUT
        for i in 0..count as usize {
            let offset = i * size_usize;
            let data = unsafe { std::slice::from_raw_parts(data_ptr.add(offset), size_usize) };
            pio_bus.write(port, data);
        }
    }
}

#[cfg(test)]
mod tests {
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
        let handle = std::thread::spawn(move || {
            #[cfg(target_arch = "x86_64")]
            c.wait_if_paused(0, || Ok(snapshot(0))).unwrap();
            #[cfg(not(target_arch = "x86_64"))]
            c.wait_if_paused();
            c.is_stopped()
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
        let handle = std::thread::spawn(move || {
            c.wait_if_paused(0, || Ok(snapshot(0))).unwrap();
        });

        control.request_pause(Duration::from_secs(1)).unwrap();
        let snapshots = control.snapshots().unwrap();

        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].id, 0);
        control.resume().unwrap();
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
}
