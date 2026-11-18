//! Forward cancellation while preparing; restore inherited dispositions for exec.
use nix::sys::signal::{signal, SigHandler, Signal};
use std::sync::atomic::{AtomicI32, Ordering};

static RECEIVED: AtomicI32 = AtomicI32::new(0);
const SIGNALS: [Signal; 4] = [Signal::SIGTERM, Signal::SIGINT, Signal::SIGHUP, Signal::SIGQUIT];
extern "C" fn receive(value: i32) {
    let _ = RECEIVED.compare_exchange(0, value, Ordering::Relaxed, Ordering::Relaxed);
}
pub(super) fn received() -> Option<i32> {
    match RECEIVED.load(Ordering::Relaxed) {
        0 => None,
        signal => Some(signal),
    }
}
pub(super) struct Guard(Vec<(Signal, SigHandler)>);
impl Guard {
    pub(super) fn install() -> Result<Self, nix::errno::Errno> {
        RECEIVED.store(0, Ordering::Relaxed);
        let mut guard = Self(Vec::with_capacity(SIGNALS.len()));
        for number in SIGNALS {
            // The handler performs only an atomic store. This standalone helper
            // installs it once, before spawning the startup process.
            let previous = unsafe { signal(number, SigHandler::Handler(receive)) }?;
            guard.0.push((number, previous));
        }
        Ok(guard)
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        for &(number, previous) in self.0.iter().rev() {
            unsafe {
                let _ = signal(number, previous);
            }
        }
    }
}
