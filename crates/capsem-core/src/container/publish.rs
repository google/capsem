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

mod broker;
mod saved;

pub struct Publisher {
    saved: Option<saved::Mappings>,
    pending: Mutex<HashMap<u64, oneshot::Sender<Result<VsockConnection>>>>,
    next_id: AtomicU64,
    incoming: Arc<Semaphore>,
    mappings: Arc<Semaphore>,
    tasks: Mutex<tokio::task::JoinSet<()>>,
    cancellation: CancellationToken,
    drain: tokio::sync::Mutex<()>,
}

impl Default for Publisher {
    fn default() -> Self {
        Self {
            saved: None,
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            incoming: Arc::new(Semaphore::new(128)),
            mappings: Arc::new(Semaphore::new(8)),
            tasks: Mutex::new(tokio::task::JoinSet::new()),
            cancellation: CancellationToken::new(),
            drain: tokio::sync::Mutex::new(()),
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
        let binary = std::env::current_exe()?.with_file_name("capsem-router");
        let (parent, child_socket) = StdUnixStream::pair()?;
        let mut child = tokio::process::Command::new(binary)
            .args(["--parent-pid", &std::process::id().to_string()])
            .env_clear()
            .current_dir("/")
            .stdin(Stdio::from(std::os::fd::OwnedFd::from(child_socket)))
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .context("start confined port router")?;
        let router_pid = child.id().context("router exited during startup")?;
        parent.set_nonblocking(true)?;
        let sender = capsem_foundation::unix::router_channel::Sender::new(parent.try_clone()?)?;
        let mut events = UnixStream::from_std(parent)?;
        tokio::time::timeout(Duration::from_secs(5), async {
            send_grant(&sender, Grant::Hello).await?;
            match Event::read(&mut events).await.context("read router startup response")? {
                Event::Ready => {}
                Event::ConfinementFailed => anyhow::bail!("port router could not install its sandbox"),
                _ => anyhow::bail!("router did not confirm confinement"),
            }
            Ok::<_, anyhow::Error>(())
        })
        .await
        .context("router startup timed out")??;
        let owner = self.clone();
        let cancellation = self.cancellation.child_token();
        let stop = cancellation.clone();
        let task = self.spawn(async move {
            let _permit = permit;
            let broker = broker::serve(owner, guest_port, listener, control, sender, events, stop.clone());
            tokio::pin!(broker);
            tokio::select! {
                result = &mut broker => {
                    if let Err(error) = result { tracing::warn!(%error, host_port, "publication router disconnected"); }
                }
                status = child.wait() => {
                    tracing::info!(?status, host_port, "publication router exited");
                    stop.cancel();
                    if let Err(error) = broker.await { tracing::debug!(%error, host_port, "publication cleanup after router exit"); }
                }
            }
            match child.try_wait() {
                Ok(Some(_)) => {}
                Ok(None) => if let Err(error) = child.kill().await { tracing::error!(%error, "router termination failed"); },
                Err(error) => tracing::error!(%error, "router exit status unavailable"),
            }
        })?;
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
                let mut header = [0; 9];
                tokio::time::timeout(Duration::from_secs(2), stream.read_exact(&mut header)).await??;
                let id = u64::from_be_bytes(header[..8].try_into().unwrap());
                let owner = owner.upgrade().context("VM router owner closed")?;
                if let Some(sender) = owner.pending.lock().unwrap().remove(&id) {
                    let result = if header[8] == 1 {
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
