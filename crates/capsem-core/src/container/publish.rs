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
    pending: Mutex<HashMap<u64, oneshot::Sender<Result<VsockConnection>>>>,
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
                if let Some(sender) = owner.pending.lock().unwrap().remove(&flow.id) {
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

    fn request(self: &Arc<Self>) -> Result<(Pending, oneshot::Receiver<Result<VsockConnection>>)> {
        let id = self
            .next_id
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| anyhow::anyhow!("publication connection ids exhausted"))?;
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, sender);
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
