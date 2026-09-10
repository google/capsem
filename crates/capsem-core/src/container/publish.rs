//! VM owner: bind declared listeners and broker data fds, never route TCP bytes.
use crate::hypervisor::VsockConnection;
use anyhow::{ensure, Context, Result};
use capsem_port_router::{send_grant, Event, Grant};
use capsem_proto::ipc::ServiceToProcess;
use std::collections::HashMap;
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

mod broker;

pub struct Publisher {
    pending: Mutex<HashMap<u64, oneshot::Sender<Result<VsockConnection>>>>,
    next_id: AtomicU64,
    incoming: Arc<Semaphore>,
    mappings: Arc<Semaphore>,
}

impl Default for Publisher {
    fn default() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            incoming: Arc::new(Semaphore::new(128)),
            mappings: Arc::new(Semaphore::new(8)),
        }
    }
}

pub struct Publication {
    pub host_port: u16,
    pub router_pid: u32,
    task: tokio::task::JoinHandle<()>,
}

impl Publication {
    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }
}

impl Drop for Publication {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Publisher {
    pub async fn publish(
        self: &Arc<Self>,
        host_port: u16,
        guest_port: u16,
        control: mpsc::Sender<ServiceToProcess>,
    ) -> Result<Publication> {
        ensure!(guest_port != 0, "guest port must be nonzero");
        let permit = self
            .mappings
            .clone()
            .try_acquire_owned()
            .context("VM publication limit reached")?;
        let listener =
            std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, host_port)).context("bind publication listener")?;
        let host_port = listener.local_addr()?.port();
        let binary = std::env::current_exe()?.with_file_name("capsem-port-router");
        let (parent, child_socket) = StdUnixStream::pair()?;
        let mut child = tokio::process::Command::new(binary)
            .args(["--parent-pid", &std::process::id().to_string()])
            .env_clear()
            .current_dir("/")
            .stdin(Stdio::from(std::os::fd::OwnedFd::from(child_socket)))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .context("start confined port router")?;
        let router_pid = child.id().context("router exited during startup")?;
        parent.set_nonblocking(true)?;
        let sender = capsem_foundation::unix::router_channel::Sender::new(parent.try_clone()?)?;
        let mut events = UnixStream::from_std(parent)?;
        tokio::time::timeout(Duration::from_secs(5), async {
            send_grant(
                &sender,
                Grant::Listen {
                    socket: listener.as_fd(),
                },
            )
            .await?;
            ensure!(
                Event::read(&mut events).await? == Event::Ready,
                "router did not confirm confinement"
            );
            Ok::<_, anyhow::Error>(())
        })
        .await
        .context("router startup timed out")??;
        drop(listener); // The companion exclusively owns the listening fd.
        let owner = self.clone();
        let task = tokio::spawn(async move {
            let _permit = permit;
            tokio::select! {
                result = broker::serve(owner, guest_port, control, sender, events) => {
                    if let Err(error) = result { tracing::warn!(%error, host_port, "publication router disconnected"); }
                }
                status = child.wait() => { tracing::info!(?status, host_port, "publication router exited"); }
            }
            // kill_on_drop and parent-watch cover cancellation and parent death.
        });
        Ok(Publication {
            host_port,
            router_pid,
            task,
        })
    }

    pub fn accept(self: &Arc<Self>, connection: VsockConnection) {
        let Ok(permit) = self.incoming.clone().try_acquire_owned() else {
            return;
        };
        let owner = self.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let result = async {
                let socket = StdUnixStream::from(connection.try_clone_fd()?);
                socket.set_nonblocking(true)?;
                let mut stream = UnixStream::from_std(socket)?;
                let mut header = [0; 9];
                tokio::time::timeout(Duration::from_secs(2), stream.read_exact(&mut header)).await??;
                let id = u64::from_be_bytes(header[..8].try_into().unwrap());
                if let Some(sender) = owner.pending.lock().unwrap().remove(&id) {
                    let result = if header[8] == 1 {
                        Ok(connection)
                    } else {
                        Err(anyhow::anyhow!("guest refused container TCP connection"))
                    };
                    let _ = sender.send(result);
                }
                Ok::<_, anyhow::Error>(())
            }
            .await;
            if let Err(error) = result {
                tracing::debug!(%error, "publication data setup refused");
            }
        });
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
