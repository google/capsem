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
use std::process::Stdio;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::net::UnixStream;
use tokio::sync::{mpsc, oneshot, Semaphore};
use tokio_util::sync::CancellationToken;

mod admission;
mod broker;
mod companion;
mod saved;
mod security;
pub use security::AuditFlow;

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
    /// The private class has its own budget: a flood of member traffic
    /// cannot starve published ports, nor the other way round.
    private_ingress: Arc<Semaphore>,
    private_setups: Arc<Semaphore>,
    private_rate: Arc<admission::SetupRate>,
    tasks: Mutex<tokio::task::JoinSet<()>>,
    cancellation: CancellationToken,
    drain: tokio::sync::Mutex<()>,
    router: tokio::sync::Mutex<Option<Arc<companion::Router>>>,
}

type GuestClose = (capsem_proto::router::FlowKey, capsem_proto::router::CloseReport);

/// Where a flow's bytes come from: a host client on a published port, or a
/// stream another VM owner handed over for a private connection. The broker
/// treats both the same way; only how each is prepared and torn down differs.
pub enum Source {
    Tcp(std::net::TcpStream),
    Stream(std::os::fd::OwnedFd),
}

impl Source {
    pub fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        match self {
            Self::Tcp(stream) => stream.as_fd(),
            Self::Stream(fd) => fd.as_fd(),
        }
    }

    /// What every source gets before it is granted: full buffers, and for
    /// TCP no Nagle and a reset on close so a dropped host client never
    /// lingers in the kernel.
    fn prepare(&self) -> std::io::Result<()> {
        capsem_foundation::unix::fd::set_stream_buffers(
            self.as_fd(),
            capsem_foundation::unix::router_stream::SOCKET_BUFFER_SIZE,
        )?;
        if let Self::Tcp(stream) = self {
            stream.set_nodelay(true)?;
            capsem_foundation::unix::fd::tcp_reset_on_close(self.as_fd())?;
        }
        Ok(())
    }

    /// End the flow now, discarding what the peer has not read.
    fn reset(&self) -> std::io::Result<()> {
        match self {
            Self::Tcp(_) => capsem_foundation::unix::fd::reset_tcp(self.as_fd()).map(|_| ()),
            Self::Stream(_) => {
                capsem_foundation::unix::fd::shutdown(self.as_fd(), capsem_foundation::unix::fd::SocketShutdown::Both)
            }
        }
    }

    /// End the flow after what was written has been delivered.
    fn close_gracefully(&self) -> std::io::Result<()> {
        match self {
            Self::Tcp(stream) => {
                capsem_foundation::unix::fd::tcp_clear_reset_on_close(self.as_fd())?;
                stream.shutdown(std::net::Shutdown::Both)
            }
            Self::Stream(_) => {
                capsem_foundation::unix::fd::shutdown(self.as_fd(), capsem_foundation::unix::fd::SocketShutdown::Both)
            }
        }
    }
}

/// One connection for a broker to set up: its source, the audit facts the
/// feeder established, the guest port to connect, and anything that must
/// stay open for as long as the flow does.
pub struct Incoming {
    pub source: Source,
    pub audit: security::AuditFlow,
    pub port: u16,
    /// The handoff channel from the source owner: closing it tells that
    /// owner the flow is over, so it is held for the flow's life.
    pub keepalive: Option<capsem_foundation::unix::router_channel::Receiver>,
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
            private_ingress: Arc::new(Semaphore::new(usize::from(budgets.private.connections))),
            private_setups: Arc::new(Semaphore::new(usize::from(budgets.private.setups))),
            private_rate: Arc::new(admission::SetupRate::new(
                budgets.private.rate_per_second,
                budgets.private.burst,
            )),
            tasks: Mutex::new(tokio::task::JoinSet::new()),
            cancellation: CancellationToken::new(),
            drain: tokio::sync::Mutex::new(()),
            router: tokio::sync::Mutex::new(None),
            budgets,
        })
    }
}

pub struct Publication {
    pub host_port: u16,
    pub router_pid: u32,
    task: tokio::task::AbortHandle,
    cancellation: CancellationToken,
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

    pub async fn publish(
        self: &Arc<Self>,
        host_port: u16,
        guest_port: u16,
        control: mpsc::Sender<ServiceToProcess>,
    ) -> Result<Publication> {
        let lifecycle = self.drain.lock().await;
        ensure!(!self.cancellation.is_cancelled(), "VM router is shutting down");
        ensure!(guest_port != 0, "guest port must be nonzero");
        ensure!(self.security.is_some(), "publication security context missing");
        let permit = self
            .mappings
            .clone()
            .try_acquire_owned()
            .context("VM publication limit reached")?;
        let listener =
            std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, host_port)).context("bind publication listener")?;
        let host_port = listener.local_addr()?.port();
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
        let incoming = self.accept_publication(listener, guest_port, stop.clone())?;
        let task = self.spawn(async move {
            let _permit = permit;
            if let Err(error) =
                broker::serve(owner, incoming, control, router, stop, capsem_router::Class::Expose).await
            {
                tracing::warn!(%error, host_port, "publication broker ended");
            }
        })?;
        drop(lifecycle);
        Ok(Publication {
            host_port,
            router_pid,
            task,
            cancellation,
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
    /// Serve private connections other owners hand over on `incoming`, for as
    /// long as the feeder keeps the channel open. The private class has its
    /// own router budget; everything else is the publication path.
    pub async fn serve_private(
        self: &Arc<Self>,
        control: mpsc::Sender<ServiceToProcess>,
        incoming: mpsc::Receiver<Incoming>,
    ) -> Result<tokio::task::AbortHandle> {
        let lifecycle = self.drain.lock().await;
        ensure!(!self.cancellation.is_cancelled(), "VM router is shutting down");
        ensure!(self.security.is_some(), "publication security context missing");
        let mut current = self.router.lock().await;
        if current.as_ref().is_none_or(|router| router.closed.is_cancelled()) {
            *current = Some(companion::start(self).await?);
        }
        let router = current.as_ref().unwrap().clone();
        drop(current);
        let owner = self.clone();
        let stop = self.cancellation.child_token();
        let task = self.spawn(async move {
            if let Err(error) =
                broker::serve(owner, incoming, control, router, stop, capsem_router::Class::Private).await
            {
                tracing::warn!(%error, "private connection broker ended");
            }
        })?;
        drop(lifecycle);
        Ok(task)
    }

    pub fn generation(&self) -> NonZeroU64 {
        self.generation
    }

    /// The audit facts of a private connection this VM is the destination
    /// of, against this owner's security authority.
    pub fn private_audit(
        &self,
        network: crate::security_engine::network::NetworkIdentity,
        source: crate::security_engine::network::NetworkVm,
        source_address: std::net::SocketAddr,
        port: u16,
    ) -> Result<security::AuditFlow> {
        let authority = self.security.clone().context("publication security context missing")?;
        Ok(security::AuditFlow::private(
            authority,
            network,
            source,
            source_address,
            port,
        ))
    }

    /// Accept host clients on a published port and hand each to a broker
    /// with its audit facts established. Accepting is the feeder's job; the
    /// broker only ever sees connections it can already account for.
    pub(super) fn accept_publication(
        self: &Arc<Self>,
        listener: tokio::net::TcpListener,
        guest_port: u16,
        stop: CancellationToken,
    ) -> Result<mpsc::Receiver<Incoming>> {
        let authority = self.security.clone().context("publication security context missing")?;
        let host_address = listener.local_addr()?;
        let publication_id = uuid::Uuid::new_v4();
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
                    source: Source::Tcp(source),
                    audit,
                    port: guest_port,
                    keepalive: None,
                };
                if feed.send(arrival).await.is_err() {
                    return;
                }
            }
        })?;
        Ok(incoming)
    }

    /// The budget a class draws on: connections held, setups in flight, and
    /// the setup rate.
    fn budget(&self, class: capsem_router::Class) -> (Arc<Semaphore>, Arc<Semaphore>, Arc<admission::SetupRate>) {
        match class {
            capsem_router::Class::Expose => (self.ingress.clone(), self.setups.clone(), self.setup_rate.clone()),
            capsem_router::Class::Private => (
                self.private_ingress.clone(),
                self.private_setups.clone(),
                self.private_rate.clone(),
            ),
        }
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
