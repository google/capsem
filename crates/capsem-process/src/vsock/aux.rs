//! Bounded auxiliary guest vsock services.

use std::os::fd::AsFd as _;
use std::sync::Arc;

use capsem_core::{read_control_msg, VsockConnection};
use capsem_proto::ipc::{ProcessToService, ServiceToProcess};
use capsem_proto::{GuestToHost, HostVsockService};
use tokio::sync::{broadcast, OwnedSemaphorePermit, Semaphore};
use tracing::{info, warn};

use super::{audit::serve_audit_records, exec_input, exec_output, streams};
use crate::helpers::clone_fd;
use crate::job_store::JobStore;

pub(super) const MAX_DEFERRED_AUX_CONNECTIONS: usize = 32;
const FIRST_AUX_FRAME_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
pub(super) const MAX_EXEC_CONNECTIONS: usize = 16;
pub(super) const MAX_AUDIT_CONNECTIONS: usize = 4;
pub(super) const MAX_LIFECYCLE_CONNECTIONS: usize = 4;

pub(super) struct AuxConnectionLimits {
    exec: Arc<Semaphore>,
    audit: Arc<Semaphore>,
    lifecycle: Arc<Semaphore>,
}

impl Default for AuxConnectionLimits {
    fn default() -> Self {
        Self {
            exec: Arc::new(Semaphore::new(MAX_EXEC_CONNECTIONS)),
            audit: Arc::new(Semaphore::new(MAX_AUDIT_CONNECTIONS)),
            lifecycle: Arc::new(Semaphore::new(MAX_LIFECYCLE_CONNECTIONS)),
        }
    }
}

impl AuxConnectionLimits {
    pub(super) fn try_acquire(&self, service: HostVsockService) -> Option<OwnedSemaphorePermit> {
        let semaphore = match service {
            HostVsockService::Exec => &self.exec,
            HostVsockService::Audit => &self.audit,
            HostVsockService::Lifecycle => &self.lifecycle,
            _ => return None,
        };
        Arc::clone(semaphore).try_acquire_owned().ok()
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn dispatch_aux_connection(
    conn: VsockConnection,
    upstream_grants: &Arc<capsem_core::net::upstream_grant::UpstreamGrantClient>,
    security_rules: &Arc<std::sync::RwLock<Arc<capsem_core::net::policy_config::SecurityRuleSet>>>,
    job_store: &Arc<JobStore>,
    db: &Arc<capsem_logger::DbWriter>,
    ipc_tx: &broadcast::Sender<ProcessToService>,
    ctrl_tx: &crate::control_queue::Sender,
    vm_id: &str,
    limits: &AuxConnectionLimits,
) {
    match HostVsockService::from_port(conn.port) {
        Some(HostVsockService::Publication) => job_store.publisher.accept(conn),
        Some(HostVsockService::Network) => streams::serve_network(conn, job_store, vm_id),
        Some(HostVsockService::SniProxy) => surrender_proxy_traffic(
            conn,
            capsem_proto::upstream_grant::ProxyTrafficService::Http,
            "HTTP",
            upstream_grants,
        ),
        Some(HostVsockService::DnsProxy) => surrender_proxy_traffic(
            conn,
            capsem_proto::upstream_grant::ProxyTrafficService::Dns,
            "DNS",
            upstream_grants,
        ),
        Some(HostVsockService::Exec) => {
            let Some(permit) = limits.try_acquire(HostVsockService::Exec) else {
                warn!("exec port: connection limit reached");
                return;
            };
            let js = Arc::clone(job_store);
            std::thread::spawn(move || {
                let _permit = permit;
                let Some(mut file) = clone_fd(&conn, "duplicate-exec-vsock") else {
                    return;
                };
                if capsem_foundation::unix::fd::set_receive_timeout(file.as_fd(), Some(FIRST_AUX_FRAME_TIMEOUT))
                    .is_err()
                {
                    return;
                }
                if let Ok(GuestToHost::ExecStarted { id, output_protocol }) = read_control_msg(&mut file) {
                    if capsem_foundation::unix::fd::set_receive_timeout(file.as_fd(), None).is_err() {
                        return;
                    }
                    info!(id, ?output_protocol, "exec port: received ExecStarted");
                    let (stream, input) = js
                        .active_execs
                        .lock()
                        .unwrap()
                        .get_mut(&id)
                        .map(|active| (active.stream.clone(), active.input_rx.take()))
                        .unwrap_or((None, None));
                    let _input_handle = exec_input::spawn_for(&conn, id, input, stream.clone(), output_protocol);
                    let capture = exec_output::read_protocol(&mut file, id, stream.as_ref(), output_protocol);
                    let total_seen = capture.stdout_bytes.saturating_add(capture.stderr_bytes);
                    let retained = capture.stdout.len().saturating_add(capture.stderr.len());
                    if total_seen > retained as u64 {
                        warn!(
                            id,
                            retained,
                            total_bytes = total_seen,
                            cap_per_lane = exec_output::EXEC_LEDGER_BODY_BYTES,
                            "exec output exceeded the ledger cap; retaining each lane's prefix"
                        );
                    }
                    // Deposit captured bytes and signal ExecDone it can
                    // proceed. notify_one stores a permit if ExecDone is
                    // not yet parked, so the common "deposit finishes
                    // first" path wakes ExecDone immediately.
                    let notify = exec_output::deposit(&js, id, capture);
                    if let Some(n) = notify {
                        n.notify_one();
                    }
                }
                drop(conn);
            });
        }
        Some(HostVsockService::Audit) => {
            let Some(permit) = limits.try_acquire(HostVsockService::Audit) else {
                warn!("audit port: connection limit reached");
                return;
            };
            let db_clone = Arc::clone(db);
            let security_rules = Arc::clone(security_rules);
            std::thread::spawn(move || {
                let _permit = permit;
                let Some(mut file) = clone_fd(&conn, "duplicate-audit-vsock") else {
                    return;
                };
                info!("audit port: connected, reading audit records");
                serve_audit_records(&mut file, &db_clone, &security_rules);
                drop(conn);
            });
        }
        Some(HostVsockService::Lifecycle) => {
            let Some(permit) = limits.try_acquire(HostVsockService::Lifecycle) else {
                warn!("lifecycle port: connection limit reached");
                return;
            };
            let itx = ipc_tx.clone();
            let ctx = ctrl_tx.clone();
            let id = vm_id.to_string();
            std::thread::spawn(move || {
                let _permit = permit;
                let Some(mut f) = clone_fd(&conn, "duplicate-lifecycle-vsock") else {
                    return;
                };
                if capsem_foundation::unix::fd::set_receive_timeout(f.as_fd(), Some(FIRST_AUX_FRAME_TIMEOUT)).is_err() {
                    return;
                }
                match read_control_msg(&mut f) {
                    Ok(GuestToHost::ShutdownRequest) => {
                        info!("guest requested shutdown via lifecycle port");
                        capsem_core::try_send!(
                            "ipc_lifecycle_shutdown",
                            itx.send(ProcessToService::ShutdownRequested { id })
                        );
                        capsem_core::try_send!(
                            "ctrl_lifecycle_shutdown",
                            ctx.blocking_send(ServiceToProcess::Shutdown)
                        );
                    }
                    Ok(GuestToHost::SuspendRequest) => {
                        info!("guest requested suspend via lifecycle port");
                        capsem_core::try_send!(
                            "ipc_lifecycle_suspend",
                            itx.send(ProcessToService::SuspendRequested { id })
                        );
                    }
                    other => {
                        // W4: a lifecycle-port frame the host doesn't recognize
                        // is exactly the bug pattern that wedged us today.
                        warn!(target: "ipc", unhandled = ?other, "VSOCK_PORT_LIFECYCLE: unknown variant; this binary may be older than its peer");
                    }
                }
                drop(conn);
            });
        }
        Some(HostVsockService::Control | HostVsockService::Terminal) => {
            warn!(
                target: "ipc",
                port = conn.port,
                service = HostVsockService::from_port(conn.port).map(HostVsockService::as_str),
                "vsock dispatch: control/terminal service reached auxiliary dispatcher; connection ignored"
            );
        }
        other => {
            warn!(
                target: "ipc",
                port = conn.port,
                service = ?other.map(HostVsockService::as_str),
                "vsock dispatch: unknown port; auxiliary connection ignored"
            );
        }
    }
}

fn surrender_proxy_traffic(
    conn: VsockConnection,
    service: capsem_proto::upstream_grant::ProxyTrafficService,
    label: &'static str,
    upstream_grants: &Arc<capsem_core::net::upstream_grant::UpstreamGrantClient>,
) {
    match conn.try_clone_fd() {
        Ok(descriptor) => {
            let grants = Arc::clone(upstream_grants);
            tokio::spawn(async move {
                if let Err(error) = grants.attach_proxy_traffic(service, descriptor).await {
                    warn!(%error, "{label} port: proxy worker refused the session descriptor");
                }
            });
        }
        Err(error) => warn!(%error, "{label} port: cannot duplicate the session descriptor"),
    }
}
