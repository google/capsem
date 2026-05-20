use std::os::unix::io::RawFd;
use std::path::PathBuf;
use std::sync::Arc;

use crate::host_state::HostStateMachine;
use crate::hypervisor::VmHandle;
use crate::net::cert_authority::CertAuthority;
use capsem_logger::DbWriter;

/// Per-VM network state: telemetry DB, CA, and upstream TLS config.
///
/// Each VM gets its own `SandboxNetworkState` that is dropped when the VM stops,
/// which prevents cross-VM interference.
pub struct SandboxNetworkState {
    pub db: Arc<DbWriter>,
    pub ca: Arc<CertAuthority>,
    /// Cached upstream TLS config, created once via `mitm_proxy::make_upstream_tls_config()`.
    pub upstream_tls: Arc<crate::net::mitm_proxy::UpstreamTlsConfig>,
}

/// Per-VM instance state (Sandbox).
///
/// Owns the hypervisor handle and all associated state for a single VM.
pub struct SandboxInstance {
    pub vm: Box<dyn VmHandle>,
    pub serial_input_fd: RawFd,
    pub vsock_terminal_fd: Option<RawFd>,
    pub vsock_control_fd: Option<RawFd>,
    pub net_state: Option<SandboxNetworkState>,
    pub state_machine: HostStateMachine,
    pub scratch_disk_path: Option<PathBuf>,
    /// Host-side file monitor. Must outlive the session -- dropping stops the watcher.
    pub fs_monitor: Option<crate::fs_monitor::FsMonitor>,
}
