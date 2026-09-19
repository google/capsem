//! Admission for blocking VM launches and exclusive hypervisor operations.

use std::sync::Mutex;

#[derive(Default)]
pub struct VmLifecycle {
    pub vz: tokio::sync::RwLock<()>,
    admission: Mutex<Admission>,
}

#[derive(Default)]
struct Admission {
    launching: usize,
    restarting: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartDenied {
    LaunchInProgress,
    ActiveVms,
    AlreadyRequested,
}

impl std::fmt::Display for RestartDenied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::LaunchInProgress => "a VM launch is in progress",
            Self::ActiveVms => "stop all active VMs before restarting the service",
            Self::AlreadyRequested => "service restart has already been requested",
        })
    }
}

impl std::error::Error for RestartDenied {}

impl VmLifecycle {
    /// Keep this permit in the blocking worker through instance registration.
    /// It must not live only in the HTTP future, which can be cancelled.
    pub fn admit(&self) -> Result<LaunchPermit<'_>, RestartDenied> {
        let mut admission = self.admission.lock().unwrap();
        if admission.restarting {
            return Err(RestartDenied::AlreadyRequested);
        }
        admission.launching += 1;
        drop(admission);
        Ok(LaunchPermit(self))
    }

    /// Check the instance registry while new launches are excluded. Refusal
    /// leaves admission open and does not alter any VM state.
    pub fn begin_restart(&self, has_active_vms: impl FnOnce() -> bool) -> Result<(), RestartDenied> {
        let mut admission = self.admission.lock().unwrap();
        if admission.restarting {
            return Err(RestartDenied::AlreadyRequested);
        }
        if admission.launching != 0 {
            return Err(RestartDenied::LaunchInProgress);
        }
        if has_active_vms() {
            return Err(RestartDenied::ActiveVms);
        }
        admission.restarting = true;
        drop(admission);
        Ok(())
    }
}

#[must_use = "hold the permit through VM instance registration"]
pub struct LaunchPermit<'a>(&'a VmLifecycle);

impl Drop for LaunchPermit<'_> {
    fn drop(&mut self) {
        self.0.admission.lock().unwrap().launching -= 1;
    }
}

#[cfg(test)]
mod tests;
