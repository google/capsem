//! VM owner: bind declared listeners and broker data fds, never route TCP bytes.
use crate::hypervisor::VsockConnection;
use anyhow::{ensure, Context, Result};
use capsem_proto::ipc::ServiceToProcess;
use capsem_router::{send_grant, Event, Grant};
use std::collections::HashMap;
use std::future::Future;
use std::net::Ipv4Addr;
use std::num::NonZeroU64;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream as StdUnixStream;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};
use tokio::io::AsyncReadExt;
use tokio::net::UnixStream;
use tokio::sync::{mpsc, oneshot, Semaphore};
use tokio_util::sync::CancellationToken;

mod admission;
mod broker;
mod companion;
mod registry;
mod saved;
mod security;
pub use security::{AuditFlow, ContainerPullRefused, ExposureRefused};

pub struct Publisher {
    security: Option<Arc<security::Authority>>,
    control_lease: Mutex<Option<CancellationToken>>,
    budgets: capsem_config::router::RouterConfig,
    /// Nonzero by type: audit identities and guest flow keys both require it,
    /// so no later stage has to unwrap it.
    generation: NonZeroU64,
    saved: Option<saved::Mappings>,
    pending: Mutex<HashMap<u64, GuestFlow>>,
    next_id: AtomicU64,
    incoming: Arc<Semaphore>,
    mappings: Arc<Semaphore>,
    ingress: Arc<Semaphore>,
    setups: Arc<Semaphore>,
    setup_rate: Arc<admission::SetupRate>,
    tasks: Mutex<tokio::task::JoinSet<()>>,
    cancellation: CancellationToken,
    drain: tokio::sync::Mutex<()>,
    router: tokio::sync::Mutex<Option<Arc<companion::Router>>>,
    declared: registry::Registry<Publication>,
}

type GuestClose = (capsem_proto::router::FlowKey, capsem_proto::router::CloseReport);

/// Where a flow's bytes come from: a host client on a published port.
pub struct Source(pub std::net::TcpStream);

impl Source {
    pub fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        self.0.as_fd()
    }

    /// What every source gets before it is granted: full buffers, no Nagle
    /// and a reset on close so a dropped host client never lingers in the
    /// kernel.
    fn prepare(&self) -> std::io::Result<()> {
        capsem_foundation::unix::fd::set_stream_buffers(
            self.as_fd(),
            capsem_foundation::unix::router_stream::SOCKET_BUFFER_SIZE,
        )?;
        self.0.set_nodelay(true)?;
        capsem_foundation::unix::fd::tcp_reset_on_close(self.as_fd())?;
        Ok(())
    }

    /// End the flow now, discarding what the peer has not read.
    fn reset(&self) -> std::io::Result<()> {
        capsem_foundation::unix::fd::reset_tcp(self.as_fd()).map(|_| ())
    }

    /// End the flow after what was written has been delivered.
    fn close_gracefully(&self) -> std::io::Result<()> {
        capsem_foundation::unix::fd::tcp_clear_reset_on_close(self.as_fd())?;
        self.0.shutdown(std::net::Shutdown::Both)
    }
}

/// One connection for a broker to set up: its source, the audit facts the
/// feeder established, and the guest port to connect.
pub struct Incoming {
    pub source: Source,
    pub audit: security::AuditFlow,
    pub port: u16,
    pub target: capsem_proto::PublicationTarget,
    /// The request shape a preview connection was admitted for; `None` for a
    /// published host port.
    pub preview: Option<capsem_proto::PreviewAdmissionKind>,
}

struct GuestFlow {
    lease: Option<CancellationToken>,
    data: Option<oneshot::Sender<Result<VsockConnection>>>,
    close: mpsc::Sender<GuestClose>,
    report: Option<capsem_proto::router::CloseReason>,
    source: std::sync::Weak<Source>,
}

impl Default for Publisher {
    fn default() -> Self {
        Self::configured(capsem_config::router::RouterConfig::default()).expect("valid default router budgets")
    }
}

impl Publisher {
    pub fn configured(budgets: capsem_config::router::RouterConfig) -> Result<Self> {
        budgets.validate().map_err(anyhow::Error::msg)?;
        Ok(Self {
            security: None,
            control_lease: Mutex::new(None),
            // The UUID variant bits make its low half nonzero; the fallback is
            // unreachable and exists so that nothing here can panic.
            generation: NonZeroU64::new(uuid::Uuid::new_v4().as_u128() as u64).unwrap_or(NonZeroU64::MIN),
            saved: None,
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            incoming: Arc::new(Semaphore::new(128)),
            mappings: Arc::new(Semaphore::new(8)),
            ingress: Arc::new(Semaphore::new(usize::from(budgets.expose.connections))),
            setups: Arc::new(Semaphore::new(usize::from(budgets.expose.setups))),
            setup_rate: Arc::new(admission::SetupRate::new(
                budgets.expose.rate_per_second,
                budgets.expose.burst,
            )),
            tasks: Mutex::new(tokio::task::JoinSet::new()),
            cancellation: CancellationToken::new(),
            drain: tokio::sync::Mutex::new(()),
            router: tokio::sync::Mutex::new(None),
            declared: registry::Registry::default(),
            budgets,
        })
    }
}

pub struct Publication {
    pub host_port: Option<u16>,
    listener: std::net::SocketAddr,
    pub router_pid: u32,
    /// The identity its admission, connections and revocation are audited under.
    pub publication_id: uuid::Uuid,
    task: tokio::task::AbortHandle,
    cancellation: CancellationToken,
    preview: Option<Arc<PreviewState>>,
}

const PREVIEW_BOOTSTRAP_LIFETIME: Duration = Duration::from_secs(30);
const PREVIEW_SESSION_LIFETIME: Duration = Duration::from_secs(15 * 60);
const PREVIEW_HANDOFF_LIFETIME: Duration = Duration::from_secs(5);
const MAX_PREVIEW_CREDENTIALS: usize = 128;

struct PreviewCredentials {
    bootstraps: HashMap<String, Instant>,
    sessions: HashMap<String, Instant>,
    handoffs: HashMap<u64, (Instant, capsem_proto::PreviewAdmissionKind)>,
}

struct PreviewState {
    incoming: mpsc::Sender<Incoming>,
    credentials: Mutex<PreviewCredentials>,
}

impl PreviewState {
    fn new(incoming: mpsc::Sender<Incoming>) -> Self {
        Self {
            incoming,
            credentials: Mutex::new(PreviewCredentials {
                bootstraps: HashMap::new(),
                sessions: HashMap::new(),
                handoffs: HashMap::new(),
            }),
        }
    }

    fn secret() -> String {
        format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
    }

    fn create_session(&self) -> Result<String> {
        let now = Instant::now();
        let mut credentials = self.credentials.lock().unwrap();
        credentials.bootstraps.retain(|_, expires| *expires > now);
        ensure!(
            credentials.bootstraps.len() < MAX_PREVIEW_CREDENTIALS,
            "preview bootstrap quota reached"
        );
        let token = Self::secret();
        credentials
            .bootstraps
            .insert(token.clone(), now + PREVIEW_BOOTSTRAP_LIFETIME);
        drop(credentials);
        Ok(token)
    }

    fn exchange(&self, token: &str) -> Result<String> {
        let now = Instant::now();
        let mut credentials = self.credentials.lock().unwrap();
        let expires = credentials
            .bootstraps
            .remove(token)
            .context("unknown, reused or expired preview bootstrap")?;
        ensure!(expires > now, "unknown, reused or expired preview bootstrap");
        credentials.sessions.retain(|_, expires| *expires > now);
        ensure!(
            credentials.sessions.len() < MAX_PREVIEW_CREDENTIALS,
            "preview session quota reached"
        );
        let session = Self::secret();
        credentials
            .sessions
            .insert(session.clone(), now + PREVIEW_SESSION_LIFETIME);
        drop(credentials);
        Ok(session)
    }

    fn admit(&self, session: &str, kind: capsem_proto::PreviewAdmissionKind) -> Result<u64> {
        let now = Instant::now();
        let mut credentials = self.credentials.lock().unwrap();
        credentials.sessions.retain(|_, expires| *expires > now);
        ensure!(
            credentials.sessions.contains_key(session),
            "unknown or expired preview session"
        );
        credentials.handoffs.retain(|_, (expires, _)| *expires > now);
        ensure!(
            credentials.handoffs.len() < MAX_PREVIEW_CREDENTIALS,
            "preview handoff quota reached"
        );
        let mut token = uuid::Uuid::new_v4().as_u128() as u64;
        if token == 0 {
            token = 1;
        }
        while credentials.handoffs.contains_key(&token) {
            token = token.checked_add(1).context("preview handoff tokens exhausted")?;
        }
        credentials
            .handoffs
            .insert(token, (now + PREVIEW_HANDOFF_LIFETIME, kind));
        drop(credentials);
        Ok(token)
    }

    fn redeem(&self, token: u64) -> Option<capsem_proto::PreviewAdmissionKind> {
        let now = Instant::now();
        let (expires, kind) = self.credentials.lock().unwrap().handoffs.remove(&token)?;
        (expires > now).then_some(kind)
    }
}

impl Publication {
    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }
}

impl Drop for Publication {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

impl Publisher {
    /// A lost control lease invalidates every current guest endpoint. Keep the
    /// declared listeners, but revoke sockets before accepting a replacement lease.
    pub fn control_lost(&self) {
        if let Some(lease) = self.control_lease.lock().unwrap().take() {
            lease.cancel();
        }
        let flows: Vec<_> = {
            let mut pending = self.pending.lock().unwrap();
            pending
                .iter_mut()
                .map(|(&id, entry)| {
                    if let Some(data) = entry.data.take() {
                        let _ = data.send(Err(anyhow::anyhow!("guest control connection closed")));
                    }
                    let close = match entry.report {
                        None | Some(capsem_proto::router::CloseReason::Complete) => {
                            entry.report = Some(capsem_proto::router::CloseReason::Cancelled);
                            Some(entry.close.clone())
                        }
                        _ => None,
                    };
                    (id, entry.source.upgrade(), close)
                })
                .collect()
        };
        for (id, source, close) in flows {
            if let Some(source) = source {
                if let Err(error) = source.reset() {
                    tracing::debug!(connection_id = id, %error, "control disconnect TCP cleanup");
                }
            }
            if let Some(close) = close {
                let flow = capsem_proto::router::FlowKey {
                    generation: self.generation.get(),
                    id,
                };
                let report = capsem_proto::router::CloseReport {
                    reason: capsem_proto::router::CloseReason::Cancelled,
                    from_source: 0,
                    to_source: 0,
                };
                if let Err(error) = close.try_send((flow, report)) {
                    tracing::error!(connection_id = id, %error, "control disconnect report admission failed");
                }
            }
        }
    }

    pub async fn shutdown(&self) {
        let _drain = self.drain.lock().await;
        self.cancellation.cancel();
        let mut tasks = std::mem::take(&mut *self.tasks.lock().unwrap());
        if tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(result) = tasks.join_next().await {
                if let Err(error) = result {
                    tracing::error!(%error, "VM router task failed during shutdown");
                }
            }
        })
        .await
        .is_err()
        {
            tracing::error!("VM router tasks exceeded shutdown deadline");
            tasks.abort_all();
            while let Some(result) = tasks.join_next().await {
                if let Err(error) = result {
                    if !error.is_cancelled() {
                        tracing::error!(%error, "VM router task failed during abort");
                    }
                }
            }
        }
    }

    fn spawn(&self, task: impl Future<Output = ()> + Send + 'static) -> Result<tokio::task::AbortHandle> {
        let mut tasks = self.tasks.lock().unwrap();
        ensure!(!self.cancellation.is_cancelled(), "VM router is shutting down");
        while let Some(result) = tasks.try_join_next() {
            if let Err(error) = result {
                tracing::error!(%error, "VM router task failed");
            }
        }
        Ok(tasks.spawn(task))
    }

    /// Open a loopback listener for `guest_port` once the VM's rules admit
    /// the exposure. A refused exposure accepts no connection.
    pub async fn publish(
        self: &Arc<Self>,
        host_port: u16,
        guest_port: u16,
        target: capsem_proto::PublicationTarget,
        control: mpsc::Sender<ServiceToProcess>,
    ) -> Result<Publication> {
        self.open(
            host_port,
            guest_port,
            target,
            control,
            crate::security_engine::network::NetworkLifecycleAction::Published,
        )
        .await
    }

    async fn open(
        self: &Arc<Self>,
        host_port: u16,
        guest_port: u16,
        target: capsem_proto::PublicationTarget,
        control: mpsc::Sender<ServiceToProcess>,
        action: crate::security_engine::network::NetworkLifecycleAction,
    ) -> Result<Publication> {
        let lifecycle = self.drain.lock().await;
        ensure!(!self.cancellation.is_cancelled(), "VM router is shutting down");
        ensure!(
            target.admits(guest_port),
            "guest port {guest_port} cannot be published into the {target:?} namespace"
        );
        let authority = self.security.clone().context("publication security context missing")?;
        let permit = self
            .mappings
            .clone()
            .try_acquire_owned()
            .context("VM publication limit reached")?;
        let listener =
            std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, host_port)).context("bind publication listener")?;
        let host_address = listener.local_addr()?;
        let host_port = host_address.port();
        // Bound, so the audited listener is the real one, but not accepting:
        // dropping it on refusal serves nothing.
        let publication_id = uuid::Uuid::new_v4();
        authority
            .admit_exposure(
                publication_id,
                host_address,
                guest_port,
                target,
                capsem_proto::PublicationAccess::LoopbackTcp,
                action,
            )
            .await?;
        listener.set_nonblocking(true)?;
        let listener = tokio::net::TcpListener::from_std(listener)?;
        let mut current = self.router.lock().await;
        if current.as_ref().is_none_or(|router| router.closed.is_cancelled()) {
            *current = Some(companion::start(self).await?);
        }
        let router = current.as_ref().unwrap().clone();
        drop(current);
        let router_pid = router.pid;
        let owner = self.clone();
        let cancellation = self.cancellation.child_token();
        let stop = cancellation.clone();
        let incoming = self.accept_publication(listener, publication_id, guest_port, target, stop.clone())?;
        let task = self.spawn(async move {
            let _permit = permit;
            if let Err(error) = broker::serve(owner, incoming, control, router, stop).await {
                tracing::warn!(%error, host_port, "publication broker ended");
            }
        })?;
        drop(lifecycle);
        Ok(Publication {
            host_port: Some(host_port),
            listener: host_address,
            router_pid,
            publication_id,
            task,
            cancellation,
            preview: None,
        })
    }

    pub fn accept(self: &Arc<Self>, connection: VsockConnection) {
        let Ok(permit) = self.incoming.clone().try_acquire_owned() else {
            return;
        };
        let owner = Arc::downgrade(self);
        let cancellation = self.cancellation.clone();
        let task = self.spawn(async move {
            let _permit = permit;
            let read = async {
                let socket = StdUnixStream::from(connection.try_clone_fd()?);
                socket.set_nonblocking(true)?;
                let mut stream = UnixStream::from_std(socket)?;
                let mut header = [0; capsem_proto::router::DATA_HEADER_SIZE];
                tokio::time::timeout(Duration::from_secs(2), stream.read_exact(&mut header)).await??;
                let (flow, connected) =
                    capsem_proto::router::FlowKey::read_data_header(header).map_err(anyhow::Error::msg)?;
                let owner = owner.upgrade().context("VM router owner closed")?;
                ensure!(flow.generation == owner.generation.get(), "stale guest data generation");
                if let Some(sender) = owner
                    .pending
                    .lock()
                    .unwrap()
                    .get_mut(&flow.id)
                    .and_then(|entry| entry.data.take())
                {
                    let result = if connected {
                        Ok(connection)
                    } else {
                        Err(anyhow::anyhow!("guest refused container TCP connection"))
                    };
                    let _ = sender.send(result);
                }
                Ok::<_, anyhow::Error>(())
            };
            let result = tokio::select! {
                result = read => result,
                _ = cancellation.cancelled() => return,
            };
            if let Err(error) = result {
                tracing::debug!(%error, "publication data setup refused");
            }
        });
        if let Err(error) = task {
            tracing::debug!(%error, "guest data arrived after router shutdown");
        }
    }

    /// The control actor ACKs every report, including stale/retired duplicates.
    /// Only an existing generation-bound lease may deliver one to a broker.
    pub fn report_close(
        &self,
        flow: capsem_proto::router::FlowKey,
        report: capsem_proto::router::CloseReport,
    ) -> Result<()> {
        if flow.generation != self.generation.get() {
            return Ok(());
        }
        let mut pending = self.pending.lock().unwrap();
        let Some(entry) = pending.get_mut(&flow.id) else {
            return Ok(());
        };
        if entry.report.is_some() {
            return Ok(());
        }
        entry.report = Some(report.reason);
        let close = entry.close.clone();
        let source = entry.source.upgrade();
        if report.reason != capsem_proto::router::CloseReason::Complete {
            if let Some(data) = entry.data.take() {
                let _ = data.send(Err(anyhow::anyhow!("guest endpoint closed during setup")));
            }
        }
        drop(pending);
        // Apply the reset before the control actor ACKs. The guest keeps its
        // VSOCK endpoint open until that ACK, preventing EOF from racing a FIN
        // through the confined copier ahead of this TCP reset.
        if report.reason != capsem_proto::router::CloseReason::Complete {
            if let Some(source) = source {
                source.reset()?;
            }
        }
        close
            .try_send((flow, report))
            .context("guest close report admission failed")
    }

    fn request(
        self: &Arc<Self>,
        source: &Arc<Source>,
        close: mpsc::Sender<GuestClose>,
    ) -> Result<(Pending, oneshot::Receiver<Result<VsockConnection>>)> {
        let id = self
            .next_id
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| anyhow::anyhow!("publication connection ids exhausted"))?;
        let (sender, receiver) = oneshot::channel();
        let lease = self.control_lease.lock().unwrap().clone();
        self.pending.lock().unwrap().insert(
            id,
            GuestFlow {
                lease: lease.clone(),
                data: Some(sender),
                close,
                report: None,
                source: Arc::downgrade(source),
            },
        );
        Ok((
            Pending {
                owner: self.clone(),
                lease,
                id,
            },
            receiver,
        ))
    }
}

impl Publisher {
    /// The owner's live publications, in host port order.
    pub fn publications(&self) -> Vec<capsem_proto::ipc::PublicationInfo> {
        self.declared.list(|entry| capsem_proto::ipc::PublicationInfo {
            id: entry.id.clone(),
            host_port: entry.host_port,
            guest_port: entry.guest_port,
            target: entry.target,
            access: entry.access,
            router_pid: entry.handle.router_pid,
        })
    }

    fn declare(
        &self,
        guest_port: u16,
        target: capsem_proto::PublicationTarget,
        publication: Publication,
    ) -> capsem_proto::ipc::PublicationInfo {
        let info = capsem_proto::ipc::PublicationInfo {
            id: publication
                .host_port
                .expect("loopback publication has a port")
                .to_string(),
            host_port: publication.host_port,
            guest_port,
            target,
            access: capsem_proto::PublicationAccess::LoopbackTcp,
            router_pid: publication.router_pid,
        };
        self.declared.insert(registry::Declared {
            id: info.id.clone(),
            host_port: info.host_port,
            guest_port,
            target,
            access: info.access,
            handle: publication,
        });
        info
    }

    /// Declare a browser-only exposure. The gateway owns the one shared
    /// loopback listener; this owner receives only admitted connected sockets.
    pub async fn declare_preview(
        self: &Arc<Self>,
        listener_port: u16,
        guest_port: u16,
        target: capsem_proto::PublicationTarget,
        control: mpsc::Sender<ServiceToProcess>,
    ) -> Result<capsem_proto::ipc::PublicationInfo> {
        let lifecycle = self.drain.lock().await;
        ensure!(!self.cancellation.is_cancelled(), "VM router is shutting down");
        ensure!(listener_port != 0, "preview listener port is missing");
        ensure!(target.admits(guest_port), "invalid preview guest port");
        let permit = self
            .mappings
            .clone()
            .try_acquire_owned()
            .context("VM publication limit reached")?;
        let publication_id = uuid::Uuid::new_v4();
        let listener = (Ipv4Addr::LOCALHOST, listener_port).into();
        let authority = self.security.clone().context("publication security context missing")?;
        authority
            .admit_exposure(
                publication_id,
                listener,
                guest_port,
                target,
                capsem_proto::PublicationAccess::HttpPreview,
                crate::security_engine::network::NetworkLifecycleAction::Published,
            )
            .await?;
        let mut current = self.router.lock().await;
        if current.as_ref().is_none_or(|router| router.closed.is_cancelled()) {
            *current = Some(companion::start(self).await?);
        }
        let router = current.as_ref().unwrap().clone();
        drop(current);
        let router_pid = router.pid;
        let (feed, incoming) = mpsc::channel(capsem_router::MAX_CONNECTIONS);
        let preview = Arc::new(PreviewState::new(feed));
        let cancellation = self.cancellation.child_token();
        let stop = cancellation.clone();
        let owner = self.clone();
        let task = self.spawn(async move {
            let _permit = permit;
            if let Err(error) = broker::serve(owner, incoming, control, router, stop).await {
                tracing::warn!(%error, %publication_id, "preview broker ended");
            }
        })?;
        let id = publication_id.to_string();
        let publication = Publication {
            host_port: None,
            listener,
            router_pid,
            publication_id,
            task,
            cancellation,
            preview: Some(preview),
        };
        let info = capsem_proto::ipc::PublicationInfo {
            id: id.clone(),
            host_port: None,
            guest_port,
            target,
            access: capsem_proto::PublicationAccess::HttpPreview,
            router_pid,
        };
        self.declared.insert(registry::Declared {
            id,
            host_port: None,
            guest_port,
            target,
            access: capsem_proto::PublicationAccess::HttpPreview,
            handle: publication,
        });
        drop(lifecycle);
        Ok(info)
    }

    pub fn create_preview_session(&self, id: &str) -> Result<String> {
        self.declared
            .with(id, |entry| {
                entry.handle.preview.as_ref().map(|preview| preview.create_session())
            })
            .flatten()
            .context("preview exposure not found")?
    }

    pub fn exchange_preview_bootstrap(&self, id: &str, token: &str) -> Result<String> {
        self.declared
            .with(id, |entry| {
                entry.handle.preview.as_ref().map(|preview| preview.exchange(token))
            })
            .flatten()
            .context("preview exposure not found")?
    }

    pub fn admit_preview_connection(
        &self,
        id: &str,
        session: &str,
        kind: capsem_proto::PreviewAdmissionKind,
    ) -> Result<u64> {
        self.declared
            .with(id, |entry| {
                entry
                    .handle
                    .preview
                    .as_ref()
                    .map(|preview| preview.admit(session, kind))
            })
            .flatten()
            .context("preview exposure not found")?
    }

    pub async fn accept_preview_handoff(self: &Arc<Self>, token: u64, source: Source) -> Result<()> {
        let found = self.declared.find_map(|entry| {
            let preview = entry.handle.preview.as_ref()?;
            let kind = preview.redeem(token)?;
            Some((
                preview.clone(),
                entry.handle.publication_id,
                entry.guest_port,
                entry.target,
                kind,
            ))
        });
        let (preview, publication_id, guest_port, target, kind) =
            found.context("unknown, reused or expired preview handoff")?;
        let authority = self.security.clone().context("publication security context missing")?;
        let listener = source.0.local_addr()?;
        let peer = source.0.peer_addr()?;
        let audit = security::AuditFlow::preview(authority, publication_id, listener, peer, guest_port, kind);
        preview
            .incoming
            .send(Incoming {
                source,
                audit,
                port: guest_port,
                target,
                preview: Some(kind),
            })
            .await
            .context("preview broker closed")
    }

    pub fn generation(&self) -> NonZeroU64 {
        self.generation
    }

    /// The audit facts of this VM's link to a network's switch, against
    /// this owner's security authority.
    pub fn private_link_audit(
        &self,
        network: crate::security_engine::network::NetworkIdentity,
        own: Ipv4Addr,
    ) -> Result<security::AuditFlow> {
        let authority = self.security.clone().context("publication security context missing")?;
        Ok(security::AuditFlow::link(authority, network, own))
    }

    /// Accept host clients on a published port and hand each to a broker
    /// with its audit facts established. Accepting is the feeder's job; the
    /// broker only ever sees connections it can already account for.
    pub(super) fn accept_publication(
        self: &Arc<Self>,
        listener: tokio::net::TcpListener,
        publication_id: uuid::Uuid,
        guest_port: u16,
        target: capsem_proto::PublicationTarget,
        stop: CancellationToken,
    ) -> Result<mpsc::Receiver<Incoming>> {
        let authority = self.security.clone().context("publication security context missing")?;
        let host_address = listener.local_addr()?;
        let (feed, incoming) = mpsc::channel(capsem_router::MAX_CONNECTIONS);
        self.spawn(async move {
            loop {
                let accepted = tokio::select! {
                    accepted = listener.accept() => accepted,
                    _ = stop.cancelled() => return,
                };
                let (source, peer) = match accepted {
                    Ok(accepted) => accepted,
                    Err(error) => {
                        tracing::warn!(%error, %host_address, "publication listener failed");
                        return;
                    }
                };
                let Ok(source) = source.into_std() else { continue };
                let audit = security::AuditFlow::new(authority.clone(), publication_id, host_address, peer, guest_port);
                let arrival = Incoming {
                    source: Source(source),
                    audit,
                    port: guest_port,
                    target,
                    preview: None,
                };
                if feed.send(arrival).await.is_err() {
                    return;
                }
            }
        })?;
        Ok(incoming)
    }
}

struct Pending {
    lease: Option<CancellationToken>,
    owner: Arc<Publisher>,
    id: u64,
}

#[cfg(test)]
mod tests;
impl Drop for Pending {
    fn drop(&mut self) {
        self.owner.pending.lock().unwrap().remove(&self.id);
    }
}
