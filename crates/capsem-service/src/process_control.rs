//! Service-owned process control policy over foundation Unix primitives.

use std::collections::HashSet;
use std::io;

use capsem_foundation::unix::process::{self, ProcessId};
pub(crate) use capsem_foundation::unix::process::{ProcessState, Signal};

/// A process probe that reports an unexpected errno once per PID and then
/// remains conservative: an unprovable exit is treated as still alive.
pub(crate) struct ProcessProbe {
    operation: &'static str,
    reported: HashSet<u32>,
}

impl ProcessProbe {
    pub(crate) fn new(operation: &'static str) -> Self {
        Self {
            operation,
            reported: HashSet::new(),
        }
    }

    pub(crate) fn state(&mut self, pid: u32) -> ProcessState {
        match probe(pid) {
            Ok(state) => state,
            Err(error) => {
                if self.reported.insert(pid) {
                    tracing::warn!(
                        operation = self.operation,
                        pid,
                        errno = error.raw_os_error(),
                        error = %error,
                        "process probe failed; conservatively retaining process"
                    );
                }
                ProcessState::Alive
            }
        }
    }

    pub(crate) fn is_gone(&mut self, pid: u32) -> bool {
        self.state(pid) == ProcessState::Gone
    }

    pub(crate) fn is_alive(&mut self, pid: u32) -> bool {
        self.state(pid) == ProcessState::Alive
    }
}

pub(crate) fn probe(pid: u32) -> io::Result<ProcessState> {
    process::probe(ProcessId::try_from(pid)?)
}

/// Signal a process and log an unexpected errno once at this operation site.
/// Expected concurrent disappearance remains a quiet typed outcome.
pub(crate) fn send_or_log(pid: u32, signal: Signal, operation: &'static str) {
    match ProcessId::try_from(pid).and_then(|pid| process::send_signal(pid, signal)) {
        Ok(_) => {}
        Err(error) => {
            tracing::warn!(
                operation,
                pid,
                signal = signal_name(signal),
                errno = error.raw_os_error(),
                error = %error,
                "process signal failed"
            );
        }
    }
}

fn signal_name(signal: Signal) -> &'static str {
    match signal {
        Signal::Terminate => "SIGTERM",
        Signal::Kill => "SIGKILL",
    }
}

#[cfg(test)]
mod tests;
