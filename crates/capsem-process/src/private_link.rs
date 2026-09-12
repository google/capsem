//! The guest end of this VM's private link, and the seat the service asks
//! it from.
//!
//! The guest's `capsem-tun` connects once at boot on VSOCK 5009 and again
//! whenever its stream ends. This owner holds the current connection and
//! nothing else: frames are switched by the network's confined process. When
//! the service links the VM to a network it says so over IPC (`LinkAttach`);
//! this VM's profile decides once, and the token it gets back is presented
//! on the handoff socket, where the answer is a duplicate of the guest
//! stream. The service keeps that connection open for the link's life; when
//! it lets go, the owner ends the guest stream so the pump reconnects.
use anyhow::{ensure, Context, Result};
use capsem_core::container::publish::{AuditFlow, Publisher};
use capsem_core::security_engine::network::{NetworkIdentity, NetworkReason};
use capsem_core::security_engine::{RuntimeSecurityEventType, SecurityEnforcementAction};
use capsem_core::VsockConnection;
use capsem_foundation::unix::router_channel::{Sender, FRAME_SIZE};
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::os::fd::AsRawFd;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::AsyncReadExt;
use tokio::sync::Notify;

/// How long a link token waits for the service to present it.
const TOKEN_LIFETIME: Duration = Duration::from_secs(8);
/// How long a presented token waits for the guest to connect: a VM linked
/// at creation is still booting.
const GUEST_DEADLINE: Duration = Duration::from_secs(60);
const MAX_PENDING: usize = 16;
const FRAME_VERSION: u8 = 1;
pub(crate) const FRAME_LINK: u8 = 5;

struct PendingLink {
    audit: AuditFlow,
    expires: Instant,
}

pub(crate) struct PrivateLink {
    guest: Mutex<Option<VsockConnection>>,
    /// Counts guest attachments, so a link lets go of the stream it was
    /// given and never a newer one.
    epoch: Mutex<u64>,
    arrived: Notify,
    pending: Mutex<HashMap<u64, PendingLink>>,
    publisher: Arc<Publisher>,
    own: Ipv4Addr,
}

impl PrivateLink {
    pub(crate) fn new(publisher: Arc<Publisher>, own: Ipv4Addr) -> Self {
        Self {
            guest: Mutex::new(None),
            epoch: Mutex::new(0),
            arrived: Notify::new(),
            pending: Mutex::new(HashMap::new()),
            publisher,
            own,
        }
    }

    /// The guest connected (again): this stream is the link from now on. The
    /// previous one, if any, ends here, which the framework object's release
    /// makes final for the guest.
    pub(crate) fn attach_guest(&self, conn: VsockConnection) {
        let previous = self.guest.lock().unwrap().replace(conn);
        *self.epoch.lock().unwrap() += 1;
        drop(previous);
        self.arrived.notify_waiters();
    }

    /// The service is linking this VM to `network`: the profile decides
    /// once, the decision is recorded, and an allowed link waits under
    /// `token` for the service to present it.
    pub(crate) async fn expect(&self, token: &str, network: NetworkIdentity) -> Result<()> {
        let token = parse_token(token)?;
        let audit = self.publisher.private_link_audit(network, self.own)?;
        let action = audit.authorize().await?;
        if action != SecurityEnforcementAction::Allow {
            let reason = match action {
                SecurityEnforcementAction::Ask => NetworkReason::ApprovalRequired,
                _ => NetworkReason::Blocked,
            };
            audit
                .record(RuntimeSecurityEventType::NetworkConnectResult, reason, 0, 0)
                .await?;
            anyhow::bail!(
                "this VM's profile {} the link",
                if action == SecurityEnforcementAction::Ask {
                    "holds for approval"
                } else {
                    "blocks"
                }
            );
        }
        let now = Instant::now();
        let mut pending = self.pending.lock().unwrap();
        pending.retain(|_, link| link.expires > now);
        let admitted = pending.len() < MAX_PENDING && !pending.contains_key(&token);
        if admitted {
            pending.insert(
                token,
                PendingLink {
                    audit,
                    expires: now + TOKEN_LIFETIME,
                },
            );
        }
        drop(pending);
        ensure!(admitted, "too many links awaiting the service, or a duplicate token");
        Ok(())
    }

    fn redeem(&self, token: u64) -> Option<PendingLink> {
        let link = self.pending.lock().unwrap().remove(&token)?;
        (link.expires > Instant::now()).then_some(link)
    }

    /// The guest stream, once there is one.
    async fn guest(&self) -> Result<(std::os::fd::OwnedFd, u64)> {
        let wait = async {
            loop {
                let arrived = self.arrived.notified();
                let held = {
                    let guest = self.guest.lock().unwrap();
                    guest
                        .as_ref()
                        .map(|conn| conn.try_clone_fd().map(|fd| (fd, *self.epoch.lock().unwrap())))
                };
                if let Some(held) = held {
                    return held;
                }
                arrived.await;
            }
        };
        tokio::time::timeout(GUEST_DEADLINE, wait)
            .await
            .context("the guest never connected its link")?
            .context("duplicate the guest link stream")
    }

    /// The service presented `token` on `socket`: answer with the guest
    /// stream and hold both for as long as the service holds the socket.
    pub(crate) async fn take(&self, token: u64, socket: std::os::unix::net::UnixStream) -> Result<()> {
        let link = self.redeem(token).context("unknown, reused or expired link token")?;
        let (stream, epoch) = self.guest().await?;
        let sender = Sender::new(socket.try_clone()?)?;
        sender
            .send(&encode_link_token(token), &[stream.as_raw_fd()])
            .await
            .context("answer the service with the guest stream")?;
        drop(stream);
        link.audit
            .record(
                RuntimeSecurityEventType::NetworkConnectResult,
                NetworkReason::Connected,
                0,
                0,
            )
            .await?;
        tracing::info!(own = %self.own, "private link granted to the network switch");
        // The service's close is the signal that the link is over.
        let mut watch = tokio::net::UnixStream::from_std(socket)?;
        let mut sink = [0u8; 64];
        while watch.read(&mut sink).await? != 0 {}
        {
            let mut guest = self.guest.lock().unwrap();
            if *self.epoch.lock().unwrap() == epoch {
                drop(guest.take());
            }
        }
        link.audit
            .record(RuntimeSecurityEventType::NetworkClose, NetworkReason::Complete, 0, 0)
            .await?;
        tracing::info!(own = %self.own, "private link released; the guest stream ends");
        Ok(())
    }
}

pub(crate) fn parse_token(text: &str) -> Result<u64> {
    ensure!(text.len() == 16, "link token must be sixteen hex digits");
    u64::from_str_radix(text, 16).context("link token is not hex")
}

pub(crate) fn encode_link_token(token: u64) -> [u8; FRAME_SIZE] {
    let mut frame = [0u8; FRAME_SIZE];
    frame[0] = FRAME_VERSION;
    frame[1] = FRAME_LINK;
    frame[2..].copy_from_slice(&token.to_be_bytes());
    frame
}

#[cfg(test)]
mod tests;
