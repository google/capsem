use std::collections::HashMap;
use std::os::unix::io::RawFd;
use std::sync::Mutex;

use capsem_core::VirtualMachine;

/// Per-VM instance state.
pub struct VmInstance {
    pub vm: VirtualMachine,
    pub serial_input_fd: RawFd,
    pub vsock_terminal_fd: Option<RawFd>,
    pub vsock_control_fd: Option<RawFd>,
}

pub struct AppState {
    pub vms: Mutex<HashMap<String, VmInstance>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            vms: Mutex::new(HashMap::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_state_has_no_vms() {
        let state = AppState::new();
        let vms = state.vms.lock().unwrap();
        assert!(vms.is_empty());
    }

    #[test]
    fn mutex_is_not_poisoned_on_creation() {
        let state = AppState::new();
        assert!(!state.vms.is_poisoned());
    }
}
