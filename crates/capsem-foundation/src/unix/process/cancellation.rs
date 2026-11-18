//! Temporary cancellation capture for standalone process supervisors.
use super::Signal;
use crate::unix::errno;
use nix::sys::signal::{signal, SigHandler};
use std::io;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

static OWNED: AtomicBool = AtomicBool::new(false);
static RECEIVED: AtomicI32 = AtomicI32::new(0);
const SIGNALS: [Signal; 4] = [Signal::Terminate, Signal::Interrupt, Signal::Hangup, Signal::Quit];

extern "C" fn receive(value: i32) {
    let _ = RECEIVED.compare_exchange(0, value, Ordering::Relaxed, Ordering::Relaxed);
}

/// Capture the first cancellation signal until restored for entrypoint exec.
/// Install at standalone process entry before spawning children. Only one guard
/// may exist; do not use alongside another signal-handling runtime.
pub struct Guard {
    previous: Vec<(Signal, SigHandler)>,
    _same_thread: PhantomData<Rc<()>>,
}

impl Guard {
    pub fn install() -> io::Result<Self> {
        OWNED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| io::Error::new(io::ErrorKind::AlreadyExists, "cancellation capture already owned"))?;
        RECEIVED.store(0, Ordering::Relaxed);
        let mut guard = Self {
            previous: Vec::with_capacity(SIGNALS.len()),
            _same_thread: PhantomData,
        };
        for number in SIGNALS {
            // SAFETY: this handler only updates a lock-free atomic. Guard
            // ownership serializes installation and rolls back partial setup.
            let previous = unsafe { signal(number.as_nix(), SigHandler::Handler(receive)) }.map_err(errno::io)?;
            guard.previous.push((number, previous));
        }
        Ok(guard)
    }

    pub fn received(&self) -> Option<Signal> {
        let number = RECEIVED.load(Ordering::Relaxed);
        SIGNALS.into_iter().find(|signal| signal.number() == number)
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        for &(number, previous) in self.previous.iter().rev() {
            // SAFETY: restore exactly the dispositions saved by this guard.
            let _ = unsafe { signal(number.as_nix(), previous) };
        }
        OWNED.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests;
