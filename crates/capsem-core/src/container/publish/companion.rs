//! One confined child per VM; route its bounded events to the owning brokers.
use super::*;
use capsem_foundation::unix::router_channel::Sender;
use std::os::fd::BorrowedFd;

struct Writer {
    sender: Sender,
    next_id: u64,
}

pub(super) struct Router {
    pub pid: u32,
    pub closed: CancellationToken,
    writer: tokio::sync::Mutex<Writer>,
    observers: Mutex<HashMap<u64, Option<mpsc::Sender<Event>>>>,
}

impl Router {
    pub(super) fn new(pid: u32, sender: Sender, closed: CancellationToken) -> Self {
        Self {
            pid,
            closed,
            writer: tokio::sync::Mutex::new(Writer { sender, next_id: 1 }),
            observers: Mutex::new(HashMap::new()),
        }
    }

    pub async fn grant(
        &self,
        source: BorrowedFd<'_>,
        destination: BorrowedFd<'_>,
        observer: mpsc::Sender<Event>,
    ) -> Result<u64> {
        let mut writer = self.writer.lock().await;
        ensure!(!self.closed.is_cancelled(), "VM router is closed");
        let id = writer.next_id;
        writer.next_id = id.checked_add(1).context("router grant ids exhausted")?;
        self.observers.lock().unwrap().insert(id, Some(observer));
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            send_grant(
                &writer.sender,
                Grant::Connected {
                    id,
                    class: capsem_router::Class::Expose,
                    source,
                    destination,
                },
            ),
        )
        .await;
        drop(writer);
        match result {
            Ok(Ok(())) => Ok(id),
            error => {
                self.closed.cancel();
                anyhow::bail!("router grant failed: {error:?}")
            }
        }
    }

    pub async fn abort(&self, id: u64) -> Result<()> {
        {
            let mut observers = self.observers.lock().unwrap();
            let Some(observer) = observers.get_mut(&id) else {
                return Ok(());
            };
            // Keep a tombstone until Closed: a late ACK after publication
            // removal is expected and must not tear down unrelated listeners.
            *observer = None;
            drop(observers);
        }
        let writer = self.writer.lock().await;
        ensure!(!self.closed.is_cancelled(), "VM router is closed");
        let result =
            tokio::time::timeout(Duration::from_secs(2), send_grant(&writer.sender, Grant::Abort { id })).await;
        drop(writer);
        match result {
            Ok(Ok(())) => Ok(()),
            error => {
                self.closed.cancel();
                anyhow::bail!("router abort failed: {error:?}")
            }
        }
    }

    pub async fn read_events(&self, mut events: UnixStream) -> Result<()> {
        loop {
            let event = tokio::select! {
                _ = self.closed.cancelled() => return Ok(()),
                event = Event::read(&mut events) => event?,
            };
            let (id, terminal) = match event {
                Event::Accepted(id) => (id, false),
                Event::Closed(id) | Event::Refused(id) => (id, true),
                Event::Ready | Event::ConfinementFailed => anyhow::bail!("unexpected router startup event"),
            };
            let observer = {
                let mut observers = self.observers.lock().unwrap();
                if terminal {
                    observers.remove(&id)
                } else {
                    observers.get(&id).cloned()
                }
                .context("router reported an unknown connection")?
            };
            if let Some(observer) = observer {
                match observer.try_send(event) {
                    Ok(()) | Err(mpsc::error::TrySendError::Closed(_)) => {}
                    Err(mpsc::error::TrySendError::Full(_)) => anyhow::bail!("router event quota exceeded"),
                }
            }
        }
    }
}

pub(super) async fn start(owner: &Publisher) -> Result<Arc<Router>> {
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
        .context("start confined VM router")?;
    let pid = child.id().context("router exited during startup")?;
    parent.set_nonblocking(true)?;
    let sender = Sender::new(parent.try_clone()?)?;
    let mut events = UnixStream::from_std(parent)?;
    tokio::time::timeout(Duration::from_secs(5), async {
        send_grant(&sender, Grant::Hello).await?;
        match Event::read(&mut events).await.context("read router startup response")? {
            Event::Ready => Ok(()),
            Event::ConfinementFailed => anyhow::bail!("router could not install its sandbox"),
            _ => anyhow::bail!("router did not confirm confinement"),
        }
    })
    .await
    .context("router startup timed out")??;
    let router = Arc::new(Router::new(pid, sender, owner.cancellation.child_token()));
    let monitor = router.clone();
    owner.spawn(async move {
        tokio::select! {
            result = monitor.read_events(events) => {
                if let Err(error) = result { tracing::warn!(%error, router_pid = pid, "VM router control failed"); }
            }
            status = child.wait() => { tracing::info!(?status, router_pid = pid, "VM router exited"); }
        }
        monitor.closed.cancel();
        monitor.observers.lock().unwrap().clear();
        match child.try_wait() {
            Ok(Some(_)) => {}
            Ok(None) => {
                if let Err(error) = child.kill().await {
                    tracing::error!(%error, "router termination failed");
                }
            }
            Err(error) => tracing::error!(%error, "router exit status unavailable"),
        }
    })?;
    Ok(router)
}

#[cfg(test)]
mod tests;
