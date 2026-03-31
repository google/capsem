//! vCPU run loop: executes guest code and dispatches MMIO exits.
//!
//! Each vCPU runs on its own OS thread. The run loop calls KVM_RUN
//! in a tight loop, handling MMIO exits by dispatching to the MMIO bus,
//! and stopping when the shutdown flag is set or a system event occurs.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use anyhow::Result;
use tracing::{debug, info, warn};

use super::mmio::MmioBus;
#[cfg(target_arch = "x86_64")]
use super::pio::PioBus;
use super::sys::{VcpuExit, VcpuFd, KVM_SYSTEM_EVENT_SHUTDOWN, KVM_SYSTEM_EVENT_RESET};

/// Spawn a vCPU run loop thread.
///
/// The thread runs KVM_RUN in a loop, dispatching MMIO exits to the bus.
/// It terminates when:
/// - `shutdown` flag is set (graceful stop)
/// - Guest triggers a system event (PSCI shutdown/reset)
/// - An unrecoverable KVM error occurs
pub(super) fn run_vcpu(
    vcpu: VcpuFd,
    mmio_bus: Arc<MmioBus>,
    #[cfg(target_arch = "x86_64")]
    pio_bus: Arc<PioBus>,
    shutdown: Arc<AtomicBool>,
) -> JoinHandle<Result<()>> {
    let vcpu_id = vcpu.id();

    std::thread::Builder::new()
        .name(format!("kvm-vcpu-{vcpu_id}"))
        .spawn(move || {
            info!(vcpu_id, "vCPU thread started");
            let result = vcpu_loop(
                &vcpu,
                &mmio_bus,
                #[cfg(target_arch = "x86_64")]
                &pio_bus,
                &shutdown,
            );
            info!(vcpu_id, "vCPU thread exiting");
            result
        })
        .expect("failed to spawn vCPU thread")
}

fn vcpu_loop(
    vcpu: &VcpuFd,
    mmio_bus: &MmioBus,
    #[cfg(target_arch = "x86_64")]
    pio_bus: &PioBus,
    shutdown: &AtomicBool,
) -> Result<()> {
    loop {
        if shutdown.load(Ordering::Relaxed) {
            debug!("vCPU {} shutdown requested", vcpu.id());
            return Ok(());
        }

        let exit = vcpu.run()?;

        match exit {
            VcpuExit::Mmio { addr, data_offset: _, len, is_write } => {
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
                if direction == 0 {
                    // KVM_EXIT_IO_IN: read from device into kvm_run buffer
                    let ptr = vcpu.io_data_mut(io.data_offset);
                    let data = unsafe {
                        std::slice::from_raw_parts_mut(ptr, size as usize)
                    };
                    pio_bus.read(port, data);
                } else {
                    // KVM_EXIT_IO_OUT: write from kvm_run buffer to device
                    let ptr = vcpu.io_data_mut(io.data_offset);
                    let data = unsafe {
                        std::slice::from_raw_parts(ptr, size as usize)
                    };
                    pio_bus.write(port, data);
                }
            }

            #[cfg(target_arch = "x86_64")]
            VcpuExit::Hlt => {
                info!("guest halted (HLT) on vCPU {}", vcpu.id());
                shutdown.store(true, Ordering::SeqCst);
                return Ok(());
            }

            #[cfg(target_arch = "x86_64")]
            VcpuExit::Shutdown => {
                warn!("guest triple-fault (shutdown) on vCPU {}", vcpu.id());
                shutdown.store(true, Ordering::SeqCst);
                return Ok(());
            }

            VcpuExit::SystemEvent { event_type } => {
                match event_type {
                    KVM_SYSTEM_EVENT_SHUTDOWN => {
                        info!("guest requested shutdown (PSCI SYSTEM_OFF)");
                        shutdown.store(true, Ordering::SeqCst);
                        return Ok(());
                    }
                    KVM_SYSTEM_EVENT_RESET => {
                        info!("guest requested reset (PSCI SYSTEM_RESET)");
                        shutdown.store(true, Ordering::SeqCst);
                        return Ok(());
                    }
                    other => {
                        warn!("unknown system event type: {other}");
                    }
                }
            }

            VcpuExit::Interrupted => {
                // Interrupted by a signal -- check shutdown and retry
                continue;
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

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::mmio::MmioDevice;
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
        assert!(iters < 10000, "thread should have stopped, ran {iters} iterations");
    }
}
