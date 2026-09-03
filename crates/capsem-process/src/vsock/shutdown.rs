//! Graceful VM stop for `ServiceToProcess::Shutdown`.
//!
//! The guest is told to shut down, then the VM is stopped once the guest
//! reports `ShutdownComplete` or `SHUTDOWN_COMPLETE_TIMEOUT_SECS` passes.
//! This replaced a fixed two-second sleep that every `capsem run` and every
//! persistent-VM stop paid in full, whether or not the guest had already
//! finished.

use super::*;
use crate::job_store::JobStore;
use crate::Shutdown;
use tokio::sync::Mutex;

type SharedVm = Arc<Mutex<Box<dyn capsem_core::hypervisor::VmHandle>>>;

/// Stop the VM once the guest reports it is done (bounded), drain the
/// background owners, and exit the process. Runs on its own task so the IPC
/// loop keeps answering while the guest winds down.
pub(super) fn spawn_stop_after_guest(js: Arc<JobStore>, vm: SharedVm, shutdown: Arc<Mutex<Shutdown>>) {
    tokio::spawn(async move {
        let bound = std::time::Duration::from_secs(proto::SHUTDOWN_COMPLETE_TIMEOUT_SECS);
        let started = std::time::Instant::now();
        let guest_reported = js.wait_shutdown_complete(bound).await;
        let waited_ms = started.elapsed().as_millis() as u64;
        if guest_reported {
            info!(waited_ms, "guest reported shutdown complete; stopping VM");
        } else {
            warn!(
                waited_ms,
                bound_ms = bound.as_millis() as u64,
                "guest did not report shutdown complete; stopping VM anyway"
            );
        }
        // channel-closed-ok: spawn_blocking JoinHandle and stop()'s
        // Result are best-effort cleanup tails; nothing waits on them.
        let _ = tokio::task::spawn_blocking(move || {
            #[cfg(target_os = "macos")]
            let _ = capsem_core::hypervisor::apple_vz::run_on_main_thread(move || vm.blocking_lock().stop());
            #[cfg(not(target_os = "macos"))]
            let _ = vm.blocking_lock().stop();
        })
        .await;
        crate::drain_background_owners(&shutdown).await;
        std::process::exit(0);
    });
}
