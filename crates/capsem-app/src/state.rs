use std::collections::HashMap;
use std::os::unix::io::RawFd;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use capsem_core::VirtualMachine;
use capsem_core::HostStateMachine;
use capsem_core::net::cert_authority::CertAuthority;
use capsem_core::net::policy::NetworkPolicy;
use capsem_core::net::telemetry::WebDb;

/// Per-VM network state: policy, telemetry DB, and connection tracking.
///
/// Each VM gets its own `VmNetworkState` that is dropped when the VM stops,
/// which prevents cross-VM interference.
pub struct VmNetworkState {
    pub policy: Arc<NetworkPolicy>,
    pub web_db: Arc<Mutex<WebDb>>,
    pub ca: Arc<CertAuthority>,
    /// Cached upstream TLS config, created once via `mitm_proxy::make_upstream_tls_config()`.
    pub upstream_tls: Arc<capsem_core::net::mitm_proxy::UpstreamTlsConfig>,
}

/// Per-VM instance state.
pub struct VmInstance {
    pub vm: VirtualMachine,
    pub serial_input_fd: RawFd,
    pub vsock_terminal_fd: Option<RawFd>,
    pub vsock_control_fd: Option<RawFd>,
    pub net_state: Option<VmNetworkState>,
    pub state_machine: HostStateMachine,
    pub scratch_disk_path: Option<PathBuf>,
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
