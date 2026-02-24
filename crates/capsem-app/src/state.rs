use std::os::unix::io::RawFd;
use std::sync::Mutex;

use capsem_core::VirtualMachine;

pub struct AppState {
    pub vm: Mutex<Option<VirtualMachine>>,
    pub serial_input_fd: Mutex<Option<RawFd>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            vm: Mutex::new(None),
            serial_input_fd: Mutex::new(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_state_has_no_vm() {
        let state = AppState::new();
        let vm = state.vm.lock().unwrap();
        assert!(vm.is_none());
    }

    #[test]
    fn new_state_has_no_serial_input_fd() {
        let state = AppState::new();
        let fd = state.serial_input_fd.lock().unwrap();
        assert!(fd.is_none());
    }

    #[test]
    fn mutex_is_not_poisoned_on_creation() {
        let state = AppState::new();
        assert!(!state.vm.is_poisoned());
        assert!(!state.serial_input_fd.is_poisoned());
    }
}
