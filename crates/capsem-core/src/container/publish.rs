//! VM owner: bind declared listeners and broker data fds, never route TCP bytes.
use crate::hypervisor::VsockConnection;
use anyhow::{ensure, Context, Result};
use capsem_proto::ipc::ServiceToProcess;
use capsem_router::{send_grant, Event, Grant};
use std::collections::HashMap;
use std::future::Future;
use std::net::Ipv4Addr;
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

pub struct Publisher {
    generation: u64,
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
}

type GuestClose = (capsem_proto::router::FlowKey, capsem_proto::router::CloseReport);

struct GuestFlow {
    data: Option<oneshot::Sender<Result<VsockConnection>>>,
    close: mpsc::Sender<GuestClose>,
    report: Option<capsem_proto::router::CloseReason>,
    source: std::sync::Weak<std::net::TcpStream>,
}

impl Default for Publisher {
    fn default() -> Self {
        Self {
            // The UUID variant bits make its low half nonzero.
            generation: uuid::Uuid::new_v4().as_u128() as u64,
            saved: None,
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            incoming: Arc::new(Semaphore::new(128)),
            mappings: Arc::new(Semaphore::new(8)),
            ingress: Arc::new(Semaphore::new(capsem_router::CONNECTIONS_PER_CLASS)),
            setups: Arc::new(Semaphore::new(8)),
            setup_rate: Arc::new(admission::SetupRate::default()),
            tasks: Mutex::new(tokio::task::JoinSet::new()),
            cancellation: CancellationToken::new(),
            drain: tokio::sync::Mutex::new(()),
            router: tokio::sync::Mutex::new(None),
        }
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
                if let Err(error) = capsem_foundation::unix::fd::reset_tcp(source.as_fd()) {
                    tracing::debug!(connection_id = id, %error, "control disconnect TCP cleanup");
                }
            }
            if let Some(close) = close {
                let flow = capsem_proto::router::FlowKey {
                    generation: self.generation,
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
        let task = self.spawn(async move {
            let _permit = permit;
            if let Err(error) = broker::serve(owner, guest_port, listener, control, router, stop).await {
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
                ensure!(flow.generation == owner.generation, "stale guest data generation");
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
        if flow.generation != self.generation {
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
                capsem_foundation::unix::fd::reset_tcp(source.as_fd())?;
            }
        }
        close
            .try_send((flow, report))
            .context("guest close report admission failed")
    }

    fn request(
        self: &Arc<Self>,
        source: &Arc<std::net::TcpStream>,
        close: mpsc::Sender<GuestClose>,
    ) -> Result<(Pending, oneshot::Receiver<Result<VsockConnection>>)> {
        let id = self
            .next_id
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| anyhow::anyhow!("publication connection ids exhausted"))?;
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().unwrap().insert(
            id,
            GuestFlow {
                data: Some(sender),
                close,
                report: None,
                source: Arc::downgrade(source),
            },
        );
        Ok((
            Pending {
                owner: self.clone(),
                id,
            },
            receiver,
        ))
    }
}

struct Pending {
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
