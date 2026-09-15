//! The service's side of one network's switch: start the confined child,
//! plug each attached VM's cable into it, learn when a port closes, and
//! stop it for good when the network retires.
//!
//! Frames never come here. A cable is granted with a duplicate of the VM's
//! guest stream, acknowledged or refused by the switch, and reported back
//! with its counters when its port closes; the service turns those into
//! attachment states and ledger rows.
//!
//! One supervisor task owns the child and the event stream; the grant
//! sender stays here. Retiring (or dropping the last handle, or the switch
//! failing) cancels one token: the supervisor stops reading, kills and
//! reaps the child, and ends; retiring also drops the grant sender and
//! waits for the supervisor, so nothing about the network outlives it.
use anyhow::{ensure, Context, Result};
use capsem_foundation::unix::router_channel::Sender;
use capsem_router::{port_id, send_grant, Event, Grant, PortReport};
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::os::fd::BorrowedFd;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::UnixStream;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::{CancellationToken, WaitForCancellationFuture};

/// Where a switch sends each closed port's report.
pub type PortReports = mpsc::Sender<(u64, PortReport)>;

/// State the supervisor shares with the host; it holds no grant sender.
struct Shared {
    pid: Option<u32>,
    /// Cancelled once the switch is gone or retiring; every port with it is dead.
    closed: CancellationToken,
    ready: CancellationToken,
    pending: Mutex<HashMap<u64, oneshot::Sender<bool>>>,
}

pub struct SwitchHost {
    shared: Arc<Shared>,
    /// Taken on retirement: the switch sees its grant channel end.
    sender: tokio::sync::Mutex<Option<Sender>>,
    supervisor: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl SwitchHost {
    /// Start the confined switch for `network`; `reports` receives every
    /// closed port's report for as long as the switch lives.
    pub async fn start(network: uuid::Uuid, reports: PortReports) -> Result<Arc<Self>> {
        let confined = super::router_process::spawn(&["--network".into()])
            .await
            .with_context(|| format!("start the switch of network {network}"))?;
        tracing::info!(switch_pid = confined.pid, %network, "network switch started");
        Ok(Self::attach(
            confined.sender,
            confined.events,
            reports,
            Some(confined.child),
        ))
    }

    /// A host over an already confirmed switch channel, owning `child` when
    /// the switch is a process.
    pub fn attach(
        sender: Sender,
        events: UnixStream,
        reports: PortReports,
        child: Option<tokio::process::Child>,
    ) -> Arc<Self> {
        let shared = Arc::new(Shared {
            pid: child.as_ref().and_then(tokio::process::Child::id),
            closed: CancellationToken::new(),
            ready: CancellationToken::new(),
            pending: Mutex::new(HashMap::new()),
        });
        let supervisor = tokio::spawn(supervise(Arc::clone(&shared), events, reports, child));
        Arc::new(Self {
            shared,
            sender: tokio::sync::Mutex::new(Some(sender)),
            supervisor: tokio::sync::Mutex::new(Some(supervisor)),
        })
    }

    pub fn pid(&self) -> Option<u32> {
        self.shared.pid
    }

    pub fn is_closed(&self) -> bool {
        self.shared.closed.is_cancelled()
    }

    /// Resolves once the switch is gone.
    pub fn closed(&self) -> WaitForCancellationFuture<'_> {
        self.shared.closed.cancelled()
    }

    /// Resolves once the switch has confirmed its confinement.
    pub async fn ready(&self) -> Result<()> {
        tokio::select! {
            _ = self.shared.ready.cancelled() => Ok(()),
            _ = self.shared.closed.cancelled() => anyhow::bail!("network switch ended before it was ready"),
        }
    }

    /// Plug one cable in as the attachment `generation` of `address`;
    /// returns the port id once the switch has taken it, or the refusal.
    ///
    /// `socket` stays borrowed until the switch answers: Darwin flushes a
    /// socket whose only reference is an in-flight SCM_RIGHTS message.
    pub async fn plug(&self, generation: u32, address: Ipv4Addr, socket: BorrowedFd<'_>) -> Result<u64> {
        ensure!(generation != 0, "attachment generations start at one");
        let port = port_id(generation, address);
        let (answer, answered) = oneshot::channel();
        self.grant(Grant::Plug { port, socket }, || {
            self.shared.pending.lock().unwrap().insert(port, answer);
        })
        .await
        .inspect_err(|_| {
            self.shared.pending.lock().unwrap().remove(&port);
        })?;
        let accepted = tokio::time::timeout(Duration::from_secs(5), answered)
            .await
            .context("switch plug acknowledgement timed out")?
            .context("network switch ended before acknowledging the plug")?;
        ensure!(accepted, "network switch refused the cable for {address}");
        Ok(port)
    }

    /// Unplug a port; its close report follows on the reports channel.
    pub async fn unplug(&self, port: u64) -> Result<()> {
        self.grant(Grant::Unplug { port }, || {}).await
    }

    /// Send one grant, running `registered` first under the same lock so an
    /// answer can never arrive before its waiter. A grant that cannot be
    /// written poisons the channel, so the switch is treated as gone.
    async fn grant(&self, grant: Grant<BorrowedFd<'_>>, registered: impl FnOnce()) -> Result<()> {
        let guard = self.sender.lock().await;
        let Some(sender) = guard.as_ref().filter(|_| !self.is_closed()) else {
            anyhow::bail!("network switch is closed");
        };
        registered();
        let sent = tokio::time::timeout(Duration::from_secs(2), send_grant(sender, grant)).await;
        drop(guard);
        if !matches!(sent, Ok(Ok(()))) {
            self.shared.closed.cancel();
            anyhow::bail!("switch grant failed: {sent:?}");
        }
        Ok(())
    }

    /// Stop the switch for good: no more grants, the process killed and
    /// reaped, every task joined. Idempotent.
    pub async fn retire(&self) {
        self.shared.closed.cancel();
        drop(self.sender.lock().await.take());
        let supervisor = self.supervisor.lock().await.take();
        if let Some(supervisor) = supervisor {
            if let Err(error) = supervisor.await {
                tracing::error!(%error, switch_pid = ?self.shared.pid, "network switch supervisor failed");
            }
        }
    }
}

impl Drop for SwitchHost {
    /// An abandoned host still stops its switch: the supervisor sees the
    /// token and kills the child, even though nobody waits for it.
    fn drop(&mut self) {
        self.shared.closed.cancel();
    }
}

async fn supervise(
    shared: Arc<Shared>,
    events: UnixStream,
    reports: PortReports,
    mut child: Option<tokio::process::Child>,
) {
    let exited = async {
        match child.as_mut() {
            Some(child) => Some(child.wait().await),
            None => std::future::pending().await,
        }
    };
    tokio::select! {
        result = read_events(&shared, events, reports) => {
            if let Err(error) = result {
                tracing::warn!(%error, switch_pid = ?shared.pid, "network switch control failed");
            }
        }
        status = exited => tracing::info!(?status, switch_pid = ?shared.pid, "network switch exited"),
        _ = shared.closed.cancelled() => {}
    }
    shared.closed.cancel();
    shared.pending.lock().unwrap().clear();
    if let Some(mut child) = child {
        if let Err(error) = child.kill().await {
            tracing::debug!(%error, switch_pid = ?shared.pid, "network switch already exited");
        }
        tracing::info!(switch_pid = ?shared.pid, "network switch stopped");
    }
}

async fn read_events(shared: &Shared, mut events: UnixStream, reports: PortReports) -> Result<()> {
    loop {
        match Event::read(&mut events).await? {
            Event::Ready => shared.ready.cancel(),
            Event::Accepted(port) => answer(shared, port, true),
            Event::Refused(port) => answer(shared, port, false),
            Event::PortClosed(port, report) => {
                if reports.send((port, report)).await.is_err() {
                    anyhow::bail!("port reports are no longer taken");
                }
            }
            Event::ConfinementFailed => anyhow::bail!("network switch lost its confinement"),
            Event::Closed(id, _) => anyhow::bail!("network switch sent a relay close for {id}"),
        }
    }
}

fn answer(shared: &Shared, port: u64, accepted: bool) {
    if let Some(answer) = shared.pending.lock().unwrap().remove(&port) {
        let _ = answer.send(accepted);
    }
}

#[cfg(test)]
mod tests;
