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

/// A registered vCPU thread's identity, held only as a `pthread_kill` target.
///
/// `libc::pthread_t` is an integer on glibc and `*mut c_void` on musl, so the
/// same field made `VcpuControl` `Sync` on one libc and not on the other -- the
/// whole vCPU thread spawn stopped compiling for musl on that alone.
///
/// The value is an opaque thread identity. It is never dereferenced here; it is
/// only handed back to `pthread_kill` to make a blocking `KVM_RUN` return
/// EINTR. Moving that between threads is sound under both libcs, which the
/// integer form got for free and the pointer form has to say out loud.
#[derive(Clone, Copy)]
struct VcpuThread(libc::pthread_t);

// Safety: an opaque thread identity that is never dereferenced, only passed to
// `pthread_kill`. See the type's documentation.
unsafe impl Send for VcpuThread {}

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
    threads: Mutex<Vec<Option<VcpuThread>>>,
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
        match self
            .lifecycle
            .compare_exchange(VCPU_RUNNING, VCPU_PAUSING, Ordering::SeqCst, Ordering::SeqCst)
        {
            Ok(_) => {}
            Err(VCPU_PAUSED) => return Ok(()),
            Err(VCPU_PAUSING) => {}
            Err(VCPU_STOPPED) => bail!("cannot pause stopped KVM VM"),
            Err(other) => bail!("cannot pause KVM VM from lifecycle state {other}"),
        }

        #[cfg(target_arch = "x86_64")]
        {
            self.snapshots.lock().expect("snapshot mutex poisoned").fill(None);
        }
        self.pause_cv.notify_all();
        self.kick_vcpus();
        let deadline = Instant::now() + timeout;
        let mut paused = self.paused_count.lock().expect("pause mutex poisoned");
        while *paused < self.vcpu_count {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                self.lifecycle.store(VCPU_RUNNING, Ordering::SeqCst);
                self.pause_cv.notify_all();
                bail!("timed out pausing KVM VM: {}/{} vCPUs parked", *paused, self.vcpu_count);
            };
            let (guard, wait) = self
                .pause_cv
                .wait_timeout(paused, remaining)
                .expect("pause condvar poisoned");
            paused = guard;
            if wait.timed_out() && *paused < self.vcpu_count {
                self.lifecycle.store(VCPU_RUNNING, Ordering::SeqCst);
                self.pause_cv.notify_all();
                bail!("timed out pausing KVM VM: {}/{} vCPUs parked", *paused, self.vcpu_count);
            }
        }
        drop(paused);
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
        *slot = Some(VcpuThread(unsafe { libc::pthread_self() }));
        drop(threads);
        Ok(VcpuThreadRegistration { control: self, vcpu_id })
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
            let ret = unsafe { libc::pthread_kill(thread.0, VCPU_KICK_SIGNAL) };
            if ret == 0 {
                kicked += 1;
            } else {
                debug!(errno = ret, "failed to kick KVM vCPU thread");
            }
        }
        drop(threads);
        kicked
    }

    #[cfg(target_arch = "x86_64")]
    pub fn snapshots(&self) -> Result<Vec<checkpoint::VcpuSnapshot>> {
        let snapshots = self.snapshots.lock().expect("snapshot mutex poisoned");
        let result = snapshots
            .iter()
            .enumerate()
            .map(|(idx, snapshot)| {
                snapshot
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("missing KVM vCPU snapshot for vCPU {idx}"))
            })
            .collect();
        drop(snapshots);
        result
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
            bail!("snapshot vCPU id mismatch: snapshot={}, vcpu={}", snapshot.id, vcpu_id);
        }
        {
            let mut snapshots = self.snapshots.lock().expect("snapshot mutex poisoned");
            let slot = snapshots
                .get_mut(vcpu_id as usize)
                .ok_or_else(|| anyhow::anyhow!("vCPU id {vcpu_id} outside snapshot table"))?;
            *slot = Some(snapshot);
            drop(snapshots);
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
        while matches!(self.lifecycle.load(Ordering::SeqCst), VCPU_PAUSING | VCPU_PAUSED) && !self.is_stopped() {
            paused = self.pause_cv.wait(paused).expect("pause condvar poisoned");
        }
        *paused = paused.saturating_sub(1);
        drop(paused);
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
            let mut vcpu = vcpu;
            info!(vcpu_id, "vCPU thread started");
            let registration = control.register_current_thread(vcpu_id)?;
            let result = vcpu_loop(
                &mut vcpu,
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
    vcpu: &mut VcpuFd,
    mmio_bus: &MmioBus,
    #[cfg(target_arch = "x86_64")] pio_bus: &PioBus,
    control: &VcpuControl,
) -> Result<()> {
    #[cfg(target_arch = "x86_64")]
    let mut exit_trace = KvmExitTrace::from_env(vcpu.id());
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

        #[cfg(target_arch = "x86_64")]
        exit_trace.record(vcpu, &exit);

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
            VcpuExit::Io { direction, port, size } => {
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
                    hardware_entry_failure_reason = format_args!("{hardware_entry_failure_reason:#x}"),
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
struct KvmExitTrace {
    enabled: bool,
    vcpu_id: u32,
    count: u64,
    next_summary: u64,
}

#[cfg(target_arch = "x86_64")]
impl KvmExitTrace {
    fn from_env(vcpu_id: u32) -> Self {
        let enabled = std::env::var_os("CAPSEM_KVM_TRACE_EXITS").is_some();
        Self {
            enabled,
            vcpu_id,
            count: 0,
            next_summary: 10_000,
        }
    }

    fn record(&mut self, vcpu: &VcpuFd, exit: &VcpuExit) {
        if !self.enabled {
            return;
        }
        self.count = self.count.saturating_add(1);
        if self.count <= 64 || self.count >= self.next_summary {
            let rip = vcpu.get_regs().ok().map(|regs| regs.rip);
            warn!(
                event_name = "kvm.vcpu.exit_trace",
                vcpu_id = self.vcpu_id,
                exits = self.count,
                rip = rip.map(|value| format!("{value:#x}")).as_deref().unwrap_or("unknown"),
                exit = ?exit,
                "KVM vCPU exit trace"
            );
            if self.count >= self.next_summary {
                self.next_summary = self.next_summary.saturating_add(10_000);
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
fn dispatch_pio(pio_bus: &PioBus, direction: u8, port: u16, size: u8, count: u32, data_ptr: *mut u8) {
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
mod tests;
