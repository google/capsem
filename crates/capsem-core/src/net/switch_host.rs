//! The service's side of one network's switch: start the confined child,
//! hand it each member's link stream, learn when a link ends.
//!
//! Frames never come here. A link is granted with a duplicate of the
//! member's stream, acknowledged or refused by the switch, and reported back
//! with its counts when it closes; the network registry turns those into
//! membership states and ledger rows.
use anyhow::{ensure, Context, Result};
use capsem_foundation::unix::router_channel::Sender;
use capsem_router::{link_id, send_grant, CloseReport, Event, Grant};
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::os::fd::BorrowedFd;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::UnixStream;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

pub struct SwitchHost {
    pub pid: u32,
    /// Cancelled once the switch is gone; every link with it is dead.
    pub closed: CancellationToken,
    ready: CancellationToken,
    sender: tokio::sync::Mutex<(Sender, u32)>,
    pending: Mutex<HashMap<u64, oneshot::Sender<bool>>>,
}

impl SwitchHost {
    /// Start the confined switch for `network`; `closed` receives every
    /// link's close report for as long as the switch lives.
    pub async fn start(network: uuid::Uuid, closed: mpsc::Sender<(u64, CloseReport)>) -> Result<Arc<Self>> {
        let confined = super::router_process::spawn(&["--switch".into()])
            .await
            .with_context(|| format!("start the switch of network {network}"))?;
        let mut child = confined.child;
        let host = Self::attach(confined.pid, confined.sender, confined.events, closed);
        let watched = Arc::clone(&host);
        tokio::spawn(async move {
            let status = child.wait().await;
            tracing::info!(?status, switch_pid = watched.pid, %network, "network switch exited");
            watched.closed.cancel();
        });
        Ok(host)
    }

    /// A host over an already confirmed switch channel.
    pub fn attach(pid: u32, sender: Sender, events: UnixStream, closed: mpsc::Sender<(u64, CloseReport)>) -> Arc<Self> {
        let host = Arc::new(Self {
            pid,
            closed: CancellationToken::new(),
            ready: CancellationToken::new(),
            sender: tokio::sync::Mutex::new((sender, 0)),
            pending: Mutex::new(HashMap::new()),
        });
        let reader = Arc::clone(&host);
        tokio::spawn(async move {
            if let Err(error) = reader.read_events(events, closed).await {
                tracing::warn!(%error, switch_pid = reader.pid, "network switch control failed");
            }
            reader.closed.cancel();
            reader.pending.lock().unwrap().clear();
        });
        host
    }

    /// Resolves once the switch has confirmed its confinement.
    pub async fn ready(&self) -> Result<()> {
        tokio::select! {
            _ = self.ready.cancelled() => Ok(()),
            _ = self.closed.cancelled() => anyhow::bail!("network switch ended before it was ready"),
        }
    }

    async fn read_events(&self, mut events: UnixStream, closed: mpsc::Sender<(u64, CloseReport)>) -> Result<()> {
        loop {
            match Event::read(&mut events).await? {
                Event::Ready => self.ready.cancel(),
                event @ (Event::Accepted(_) | Event::Refused(_)) => {
                    let (id, accepted) = match event {
                        Event::Accepted(id) => (id, true),
                        Event::Refused(id) => (id, false),
                        _ => unreachable!(),
                    };
                    if let Some(answer) = self.pending.lock().unwrap().remove(&id) {
                        let _ = answer.send(accepted);
                    }
                }
                Event::Closed(id, report) => {
                    if closed.send((id, report)).await.is_err() {
                        anyhow::bail!("close reports are no longer taken");
                    }
                }
                Event::ConfinementFailed => anyhow::bail!("network switch lost its confinement"),
            }
        }
    }

    /// Grant one member's stream; returns the link id once the switch has
    /// taken it, or the refusal.
    pub async fn link(&self, address: Ipv4Addr, socket: BorrowedFd<'_>) -> Result<u64> {
        let (answer, answered) = oneshot::channel();
        let mut sender = self.sender.lock().await;
        ensure!(!self.closed.is_cancelled(), "network switch is closed");
        sender.1 = sender.1.checked_add(1).context("switch link ids exhausted")?;
        let id = link_id(sender.1, address);
        self.pending.lock().unwrap().insert(id, answer);
        let sent = tokio::time::timeout(
            Duration::from_secs(2),
            send_grant(&sender.0, Grant::Link { id, socket }),
        )
        .await;
        drop(sender);
        if !matches!(sent, Ok(Ok(()))) {
            self.pending.lock().unwrap().remove(&id);
            self.closed.cancel();
            anyhow::bail!("switch link grant failed: {sent:?}");
        }
        let accepted = tokio::time::timeout(Duration::from_secs(5), answered)
            .await
            .context("switch link acknowledgement timed out")?
            .context("network switch ended before acknowledging the link")?;
        ensure!(accepted, "network switch refused the link for {address}");
        Ok(id)
    }

    /// End a link; its close report follows on the `closed` channel.
    pub async fn unlink(&self, id: u64) -> Result<()> {
        let sender = self.sender.lock().await;
        ensure!(!self.closed.is_cancelled(), "network switch is closed");
        let sent = tokio::time::timeout(Duration::from_secs(2), send_grant(&sender.0, Grant::Abort { id })).await;
        drop(sender);
        sent.context("switch unlink timed out")??;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
