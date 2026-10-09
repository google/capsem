//! Generation-bound connected-socket grants for one VM owner.
//!
//! The coordinator selects and opens upstream sockets from its trusted active
//! policy snapshot, then passes the connected descriptor over the inherited
//! socketpair. Payload bytes never transit the coordinator. The worker cannot
//! name a DNS server or connect an address directly; TCP resolution yields an
//! opaque selection that must be connected on this same generation channel.

use std::collections::HashMap;
use std::io;
use std::os::fd::{AsFd as _, AsRawFd as _, OwnedFd};
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use capsem_core::net::mitm_proxy::protocol::Protocol;
use capsem_core::net::mitm_proxy::UpstreamTarget;
use capsem_core::net::policy_config::CompiledActivePolicy;
use capsem_core::net::upstream_address::UpstreamResolver;
use capsem_foundation::unix::fd::{self, SocketShutdown};
use capsem_foundation::unix::router_channel::{DescriptorReceiver, DescriptorSender};
use capsem_proto::upstream_grant::{
    decode_upstream_grant_request, encode_upstream_grant_response, UpstreamDescriptorKind, UpstreamGrantDenial,
    UpstreamGrantRequest, UpstreamGrantResponse, UpstreamProtocol, UPSTREAM_GRANT_FRAME_SIZE, UPSTREAM_GRANT_MAX_FDS,
};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, warn};

use crate::instance::WorkerGrant;

const MAX_SELECTIONS: usize = 64;
const MAX_ACTIVE_GRANTS: usize = 256;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(1);

type WireSender = DescriptorSender<UPSTREAM_GRANT_FRAME_SIZE, UPSTREAM_GRANT_MAX_FDS>;
type WireReceiver = DescriptorReceiver<UPSTREAM_GRANT_FRAME_SIZE, UPSTREAM_GRANT_MAX_FDS>;

pub(crate) struct BrokerPolicy {
    digest: String,
    runtime: Arc<CompiledActivePolicy>,
}

#[derive(Clone)]
pub(crate) struct PolicyPublisher {
    updates: Option<mpsc::Sender<PolicyUpdate>>,
}
pub(crate) struct PendingBroker {
    coordinator: UnixStream,
    worker: UnixStream,
    initial_policy: Arc<BrokerPolicy>,
    updates: mpsc::Receiver<PolicyUpdate>,
    session_dir: Option<PathBuf>,
}

struct PolicyUpdate {
    policy: Arc<BrokerPolicy>,
    applied: oneshot::Sender<Result<(), String>>,
}

struct Selection {
    policy_digest: String,
    target: UpstreamTarget,
}

struct ActiveGrant {
    descriptor: OwnedFd,
    kind: UpstreamDescriptorKind,
    adopted: bool,
}

impl BrokerPolicy {
    pub(crate) fn new(digest: String, runtime: Arc<CompiledActivePolicy>) -> Arc<Self> {
        Arc::new(Self { digest, runtime })
    }
}

impl PolicyPublisher {
    pub(crate) async fn publish(&self, policy: Arc<BrokerPolicy>) -> Result<(), String> {
        let Some(updates) = &self.updates else {
            return Ok(());
        };
        let (applied, confirmation) = oneshot::channel();
        updates
            .send(PolicyUpdate { policy, applied })
            .await
            .map_err(|_| "upstream policy broker stopped".to_string())?;
        confirmation
            .await
            .map_err(|_| "upstream policy broker stopped before applying the policy".to_string())?
    }
}

impl PendingBroker {
    pub(crate) fn pair(initial_policy: Arc<BrokerPolicy>) -> io::Result<(Self, PolicyPublisher)> {
        let (coordinator, worker) = UnixStream::pair()?;
        let (updates_tx, updates) = mpsc::channel(1);
        Ok((
            Self {
                coordinator,
                worker,
                initial_policy,
                updates,
                session_dir: None,
            },
            PolicyPublisher {
                updates: Some(updates_tx),
            },
        ))
    }

    pub(crate) fn pair_for_session(
        initial_policy: Arc<BrokerPolicy>,
        session_dir: PathBuf,
    ) -> io::Result<(Self, PolicyPublisher)> {
        let (mut pending, publisher) = Self::pair(initial_policy)?;
        pending.session_dir = Some(session_dir);
        Ok((pending, publisher))
    }

    pub(crate) fn worker_stdio(&self) -> io::Result<std::process::Stdio> {
        let descriptor: OwnedFd = self.worker.try_clone()?.into();
        Ok(std::process::Stdio::from(descriptor))
    }

    pub(crate) fn start(self, authority: WorkerGrant) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            if let Err(error) = run(
                self.coordinator,
                self.initial_policy,
                self.updates,
                authority,
                self.session_dir,
            )
            .await
            {
                debug!(%error, "upstream broker stopped");
            }
        })
    }
}

async fn run(
    socket: UnixStream,
    mut policy: Arc<BrokerPolicy>,
    mut updates: mpsc::Receiver<PolicyUpdate>,
    authority: WorkerGrant,
    session_dir: Option<PathBuf>,
) -> Result<(), String> {
    let requests =
        WireReceiver::new(socket.try_clone().map_err(|error| error.to_string())?).map_err(|error| error.to_string())?;
    let responses = WireSender::new(socket).map_err(|error| error.to_string())?;
    let (request_tx, mut request_rx) = mpsc::channel(1);
    let request_pump = tokio::spawn(async move {
        loop {
            let frame = requests.recv().await;
            let complete = frame.is_err();
            if request_tx.send(frame).await.is_err() || complete {
                return;
            }
        }
    });
    let mut selections = HashMap::new();
    let mut active = HashMap::new();
    let mut next_resource_id = 1_u64;
    let mut awaiting_adoption = None;
    let result = async {
        'requests: loop {
            let frame = tokio::select! {
                biased;
                () = authority.revoked() => break Err("worker authority revoked".to_string()),
                update = updates.recv() => {
                    apply_policy_update(
                        update.ok_or_else(|| "upstream policy publisher closed".to_string())?,
                        &mut policy,
                        &mut selections,
                        &mut active,
                        &mut awaiting_adoption,
                    )?;
                    continue;
                }
                frame = request_rx.recv() => frame
                    .ok_or_else(|| "upstream grant request pump stopped".to_string())?
                    .map_err(|error| format!("receive upstream grant request: {error}"))?,
            };
            if !frame.fds.is_empty() {
                break Err("worker sent an upstream descriptor".into());
            }
            let request = decode_upstream_grant_request(&frame.bytes)
                .map_err(|error| format!("decode upstream grant request: {error:#}"))?;
            if let Some(expected) = awaiting_adoption {
                if request != (UpstreamGrantRequest::Adopted { grant_id: expected }) {
                    break Err(format!(
                        "descriptor grant {expected} was not adopted before the next request"
                    ));
                }
            }
            match request {
                UpstreamGrantRequest::ResolveTcp {
                    request_id,
                    protocol,
                    host,
                    port,
                } => {
                    let current = Arc::clone(&policy);
                    selections.retain(|_, selection: &mut Selection| selection.policy_digest == current.digest);
                    if selections.len() >= MAX_SELECTIONS {
                        send_denied(&responses, request_id, UpstreamGrantDenial::Capacity).await?;
                        continue;
                    }
                    if protocol == UpstreamProtocol::Http
                        && !current.runtime.network.http_upstream_ports.is_empty()
                        && !current.runtime.network.http_upstream_ports.contains(&port)
                    {
                        send_denied(&responses, request_id, UpstreamGrantDenial::NotAllowed).await?;
                        continue;
                    }
                    let guest_protocol = core_protocol(protocol);
                    let resolver = UpstreamResolver::system();
                    let target = tokio::select! {
                        biased;
                        () = authority.revoked() => break Err("worker authority revoked".to_string()),
                        update = updates.recv() => {
                            apply_policy_update(
                                update.ok_or_else(|| "upstream policy publisher closed".to_string())?,
                                &mut policy,
                                &mut selections,
                                &mut active,
                                &mut awaiting_adoption,
                            )?;
                            continue 'requests;
                        }
                        target = UpstreamTarget::resolve(
                            &resolver,
                            &current.runtime.network,
                            &host,
                            port,
                        ) => target,
                    };
                    if matches!(target, UpstreamTarget::Unresolved(_)) {
                        send_denied(&responses, request_id, UpstreamGrantDenial::ResolveFailed).await?;
                        continue;
                    }
                    let selection_id = allocate_id(&mut next_resource_id)?;
                    let judged_ip = target.judged_ip(&host);
                    let effective_protocol = wire_protocol(target.protocol(guest_protocol))?;
                    selections.insert(
                        selection_id,
                        Selection {
                            policy_digest: current.digest.clone(),
                            target,
                        },
                    );
                    send_response(
                        &responses,
                        &UpstreamGrantResponse::TcpResolved {
                            request_id,
                            selection_id,
                            protocol: effective_protocol,
                            judged_ip,
                            policy_digest: current.digest.clone(),
                        },
                        None,
                    )
                    .await?;
                }
                UpstreamGrantRequest::ConnectTcp {
                    request_id,
                    selection_id,
                } => {
                    if active.len() >= MAX_ACTIVE_GRANTS {
                        send_denied(&responses, request_id, UpstreamGrantDenial::Capacity).await?;
                        continue;
                    }
                    let current_digest = policy.digest.clone();
                    let Some(selection) = selections.remove(&selection_id) else {
                        send_denied(&responses, request_id, UpstreamGrantDenial::InvalidResource).await?;
                        continue;
                    };
                    if selection.policy_digest != current_digest {
                        send_denied(&responses, request_id, UpstreamGrantDenial::InvalidResource).await?;
                        continue;
                    }
                    let connected = tokio::select! {
                        biased;
                        () = authority.revoked() => break Err("worker authority revoked".to_string()),
                        update = updates.recv() => {
                            apply_policy_update(
                                update.ok_or_else(|| "upstream policy publisher closed".to_string())?,
                                &mut policy,
                                &mut selections,
                                &mut active,
                                &mut awaiting_adoption,
                            )?;
                            continue 'requests;
                        }
                        result = tokio::time::timeout(CONNECT_TIMEOUT, selection.target.connect()) => result,
                    };
                    let stream = match connected {
                        Ok(Ok((stream, _pinned))) => stream,
                        Ok(Err(error)) => {
                            warn!(selection_id, %error, "upstream broker TCP connect failed");
                            send_denied(&responses, request_id, UpstreamGrantDenial::ConnectFailed).await?;
                            continue;
                        }
                        Err(_) => {
                            warn!(selection_id, "upstream broker TCP connect timed out");
                            send_denied(&responses, request_id, UpstreamGrantDenial::ConnectFailed).await?;
                            continue;
                        }
                    };
                    let descriptor: OwnedFd = stream
                        .into_std()
                        .map_err(|error| format!("adopt connected TCP stream: {error}"))?
                        .into();
                    let grant_id = allocate_id(&mut next_resource_id)?;
                    active.insert(
                        grant_id,
                        ActiveGrant {
                            descriptor,
                            kind: UpstreamDescriptorKind::Tcp,
                            adopted: false,
                        },
                    );
                    send_active_grant(
                        &responses,
                        &active,
                        request_id,
                        grant_id,
                        UpstreamDescriptorKind::Tcp,
                        &current_digest,
                    )
                    .await?;
                    awaiting_adoption = Some(grant_id);
                }
                UpstreamGrantRequest::OpenDns {
                    request_id,
                    upstream_index,
                } => {
                    if active.len() >= MAX_ACTIVE_GRANTS {
                        send_denied(&responses, request_id, UpstreamGrantDenial::Capacity).await?;
                        continue;
                    }
                    let current = Arc::clone(&policy);
                    let Some(upstream) = current.runtime.dns_upstreams.get(usize::from(upstream_index)).copied() else {
                        send_denied(&responses, request_id, UpstreamGrantDenial::NotConfigured).await?;
                        continue;
                    };
                    let bind = if upstream.is_ipv6() { "[::]:0" } else { "0.0.0.0:0" };
                    let socket = tokio::select! {
                        biased;
                        () = authority.revoked() => break Err("worker authority revoked".to_string()),
                        update = updates.recv() => {
                            apply_policy_update(
                                update.ok_or_else(|| "upstream policy publisher closed".to_string())?,
                                &mut policy,
                                &mut selections,
                                &mut active,
                                &mut awaiting_adoption,
                            )?;
                            continue 'requests;
                        }
                        result = tokio::net::UdpSocket::bind(bind) => result,
                    }
                    .map_err(|error| format!("bind broker DNS socket: {error}"))?;
                    let connected = tokio::select! {
                        biased;
                        () = authority.revoked() => break Err("worker authority revoked".to_string()),
                        update = updates.recv() => {
                            apply_policy_update(
                                update.ok_or_else(|| "upstream policy publisher closed".to_string())?,
                                &mut policy,
                                &mut selections,
                                &mut active,
                                &mut awaiting_adoption,
                            )?;
                            continue 'requests;
                        }
                        result = socket.connect(upstream) => result,
                    };
                    if let Err(error) = connected {
                        warn!(%upstream, %error, "upstream broker DNS connect failed");
                        send_denied(&responses, request_id, UpstreamGrantDenial::ConnectFailed).await?;
                        continue;
                    }
                    let descriptor: OwnedFd = socket
                        .into_std()
                        .map_err(|error| format!("adopt connected DNS socket: {error}"))?
                        .into();
                    let grant_id = allocate_id(&mut next_resource_id)?;
                    active.insert(
                        grant_id,
                        ActiveGrant {
                            descriptor,
                            kind: UpstreamDescriptorKind::DnsUdp,
                            adopted: false,
                        },
                    );
                    send_active_grant(
                        &responses,
                        &active,
                        request_id,
                        grant_id,
                        UpstreamDescriptorKind::DnsUdp,
                        &current.digest,
                    )
                    .await?;
                    awaiting_adoption = Some(grant_id);
                }
                UpstreamGrantRequest::Adopted { grant_id } => {
                    let Some(grant) = active.get_mut(&grant_id) else {
                        break Err(format!("adopted unknown descriptor grant {grant_id}"));
                    };
                    if grant.adopted || awaiting_adoption != Some(grant_id) {
                        break Err(format!("invalid descriptor adoption {grant_id}"));
                    }
                    grant.adopted = true;
                    awaiting_adoption = None;
                }
                UpstreamGrantRequest::Release { resource_id } => {
                    if selections.remove(&resource_id).is_none() {
                        let Some(grant) = active.remove(&resource_id) else {
                            break Err(format!("released unknown upstream resource {resource_id}"));
                        };
                        if !grant.adopted {
                            break Err(format!("released unadopted descriptor grant {resource_id}"));
                        }
                        revoke_grant(resource_id, grant)?;
                    }
                }
                UpstreamGrantRequest::SetGuestMode {
                    request_id,
                    relative_path,
                    mode,
                } => {
                    let Some(session_dir) = session_dir.as_deref() else {
                        send_denied(&responses, request_id, UpstreamGrantDenial::NotConfigured).await?;
                        continue;
                    };
                    match set_guest_mode(session_dir, &relative_path, mode) {
                        Ok(()) => {
                            send_response(&responses, &UpstreamGrantResponse::GuestModeSet { request_id }, None)
                                .await?;
                        }
                        Err(error) => {
                            warn!(%error, "guest mode broker refused metadata change");
                            send_denied(&responses, request_id, UpstreamGrantDenial::NotAllowed).await?;
                        }
                    }
                }
            }
        }
    }
    .await;
    request_pump.abort();
    let _ = request_pump.await;
    let cleanup = revoke_active(active);
    match (result, cleanup) {
        (Ok(()), cleanup) => cleanup,
        (Err(error), Ok(())) => Err(error),
        (Err(error), Err(cleanup)) => Err(format!("{error}; {cleanup}")),
    }
}

fn set_guest_mode(session_dir: &Path, relative_path: &[u8], mode: u16) -> io::Result<()> {
    let root = capsem_foundation::unix::contained::ContainedDir::open_root(session_dir)?
        .descend(std::ffi::OsStr::new(capsem_core::GUEST_SHARE_DIR))?;
    let path = Path::new(std::ffi::OsStr::from_bytes(relative_path));
    root.set_relative_mode(path, u32::from(mode))
}

async fn send_active_grant(
    sender: &WireSender,
    active: &HashMap<u64, ActiveGrant>,
    request_id: u64,
    grant_id: u64,
    kind: UpstreamDescriptorKind,
    policy_digest: &str,
) -> Result<(), String> {
    let grant = active.get(&grant_id).ok_or("new descriptor grant disappeared")?;
    send_response(
        sender,
        &UpstreamGrantResponse::DescriptorGranted {
            request_id,
            grant_id,
            kind,
            policy_digest: policy_digest.to_owned(),
        },
        Some(&grant.descriptor),
    )
    .await
}

async fn send_denied(sender: &WireSender, request_id: u64, reason: UpstreamGrantDenial) -> Result<(), String> {
    send_response(sender, &UpstreamGrantResponse::Denied { request_id, reason }, None).await
}

async fn send_response(
    sender: &WireSender,
    response: &UpstreamGrantResponse,
    descriptor: Option<&OwnedFd>,
) -> Result<(), String> {
    let bytes =
        encode_upstream_grant_response(response).map_err(|error| format!("encode grant response: {error:#}"))?;
    let descriptors = descriptor.map_or_else(Vec::new, |fd| vec![fd.as_raw_fd()]);
    tokio::time::timeout(RESPONSE_TIMEOUT, sender.send(&bytes, &descriptors))
        .await
        .map_err(|_| "send upstream grant response timed out".to_string())?
        .map_err(|error| format!("send upstream grant response: {error}"))?;
    Ok(())
}

fn allocate_id(next: &mut u64) -> Result<u64, String> {
    let id = *next;
    if id == 0 {
        return Err("upstream broker exhausted resource ids".into());
    }
    *next = next.checked_add(1).unwrap_or(0);
    Ok(id)
}

fn apply_policy_update(
    update: PolicyUpdate,
    current: &mut Arc<BrokerPolicy>,
    selections: &mut HashMap<u64, Selection>,
    active: &mut HashMap<u64, ActiveGrant>,
    awaiting_adoption: &mut Option<u64>,
) -> Result<(), String> {
    *current = update.policy;
    selections.clear();
    let result = revoke_active(std::mem::take(active));
    *awaiting_adoption = None;
    let _ = update.applied.send(result.clone());
    result
}

fn core_protocol(protocol: UpstreamProtocol) -> Protocol {
    match protocol {
        UpstreamProtocol::Http => Protocol::Http,
        UpstreamProtocol::Tls => Protocol::Tls,
    }
}

fn wire_protocol(protocol: Protocol) -> Result<UpstreamProtocol, String> {
    match protocol {
        Protocol::Http => Ok(UpstreamProtocol::Http),
        Protocol::Tls => Ok(UpstreamProtocol::Tls),
        Protocol::McpFrame | Protocol::Unknown => Err("invalid upstream broker protocol".into()),
    }
}

fn revoke_active(active: HashMap<u64, ActiveGrant>) -> Result<(), String> {
    let mut failures = Vec::new();
    for (grant_id, grant) in active {
        if let Err(error) = revoke_grant(grant_id, grant) {
            failures.push(error);
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

fn revoke_grant(grant_id: u64, grant: ActiveGrant) -> Result<(), String> {
    let result = match grant.kind {
        UpstreamDescriptorKind::Tcp => fd::reset_tcp(grant.descriptor.as_fd()).map(|_| ()),
        UpstreamDescriptorKind::DnsUdp => fd::shutdown(grant.descriptor.as_fd(), SocketShutdown::Both),
    };
    result.map_err(|error| format!("revoke upstream descriptor {grant_id}: {error}"))
}

#[cfg(test)]
pub(crate) fn test_policy_publisher() -> PolicyPublisher {
    PolicyPublisher { updates: None }
}

#[cfg(test)]
mod tests;
