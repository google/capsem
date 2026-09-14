//! This VM's network cables: one per network it is plugged into.
//!
//! When the service plugs the VM into a network it says so over IPC
//! (`LinkAttach`): this VM's profile decides once, the owner gives the
//! network a cable id and tells the guest to bring that cable up with the
//! network's address (`PlugCable`), and the token it answers with is
//! presented on the handoff socket, where the reply is a duplicate of that
//! cable's guest stream. The guest's pump for the cable connects on VSOCK
//! 5009, opening with the cable id, and again whenever its stream ends. This
//! owner holds each cable's current connection and nothing else: frames are
//! switched by the network's confined process. The service keeps the handoff
//! connection open for the port's life; when it lets go, the owner ends that
//! cable's stream so its pump reconnects. Leaving a network (`LinkDetach`)
//! takes the cable down in the guest.
//!
//! The guest names only a cable id: which network a cable belongs to is this
//! owner's table, so a guest can at most send its own frames down another of
//! its own cables.
use anyhow::{ensure, Context, Result};
use capsem_core::container::publish::{AuditFlow, Publisher};
use capsem_core::security_engine::network::{NetworkIdentity, NetworkReason};
use capsem_core::security_engine::{RuntimeSecurityEventType, SecurityEnforcementAction};
use capsem_core::VsockConnection;
use capsem_foundation::unix::router_channel::Sender;
use capsem_proto::ipc::ServiceToProcess;
use capsem_proto::privatelink::{seat_frame, SEAT_LINK};
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::os::fd::AsRawFd;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::AsyncReadExt;
use tokio::sync::{mpsc, Notify};

/// How long a plug token waits for the service to present it.
const TOKEN_LIFETIME: Duration = Duration::from_secs(8);
/// How long a presented token waits for the pump to connect: a VM plugged
/// at creation is still booting.
const GUEST_DEADLINE: Duration = Duration::from_secs(60);
const MAX_PENDING: usize = 16;

struct PendingPlug {
    audit: AuditFlow,
    cable: u32,
    expires: Instant,
}

struct Cable {
    network: String,
    guest: Option<VsockConnection>,
    /// Counts pump connections, so a port lets go of the stream it was given
    /// and never a newer one.
    epoch: u64,
    /// The epoch of the stream a live port holds, if one does.
    held: Option<u64>,
    /// The epoch of the last stream a port let go of: the next plug takes a
    /// newer one, which the pump provides by reconnecting, never the stream
    /// the previous port may still be draining.
    released: u64,
}

#[derive(Default)]
struct Table {
    next: u32,
    cables: HashMap<u32, Cable>,
}

impl Table {
    /// Make `conn` the cable's stream, returning the one it replaces; a cable
    /// nobody plugged hands the connection back to be closed.
    fn replace_stream(
        &mut self,
        cable: u32,
        conn: VsockConnection,
    ) -> std::result::Result<Option<VsockConnection>, VsockConnection> {
        let Some(entry) = self.cables.get_mut(&cable) else {
            return Err(conn);
        };
        entry.epoch += 1;
        Ok(entry.guest.replace(conn))
    }

    fn remove_network(&mut self, network: &str) -> Option<(u32, Cable)> {
        let cable = self
            .cables
            .iter()
            .find(|(_, entry)| entry.network == network)
            .map(|(cable, _)| *cable)?;
        self.cables.remove(&cable).map(|entry| (cable, entry))
    }

    /// The cable's stream, marked held, if it is free and newer than the last
    /// one let go of.
    fn claim(&mut self, cable: u32) -> Result<Option<(std::os::fd::OwnedFd, u64)>> {
        let entry = self.cables.get_mut(&cable).context("the cable was detached")?;
        match entry.guest.as_ref() {
            Some(conn) if entry.held.is_none() && entry.epoch > entry.released => {
                let fd = conn.try_clone_fd().context("duplicate the cable's guest stream")?;
                entry.held = Some(entry.epoch);
                Ok(Some((fd, entry.epoch)))
            }
            _ => Ok(None),
        }
    }
}

pub(crate) struct Cables {
    publisher: Arc<Publisher>,
    /// Where guest instructions go: the control hub forwards them.
    control: mpsc::Sender<ServiceToProcess>,
    table: Mutex<Table>,
    arrived: Notify,
    pending: Mutex<HashMap<u64, PendingPlug>>,
}

impl Cables {
    pub(crate) fn new(publisher: Arc<Publisher>, control: mpsc::Sender<ServiceToProcess>) -> Self {
        Self {
            publisher,
            control,
            table: Mutex::new(Table::default()),
            arrived: Notify::new(),
            pending: Mutex::new(HashMap::new()),
        }
    }

    /// A pump connected (again) for `cable`: this stream is the cable from
    /// now on, and the previous one ends. A cable nobody plugged, or one
    /// already detached, gets its connection closed.
    pub(crate) fn attach_guest(&self, cable: u32, conn: VsockConnection) {
        match self.table.lock().unwrap().replace_stream(cable, conn) {
            Ok(previous) => drop(previous),
            Err(refused) => {
                tracing::warn!(cable, "network: stream for a cable this owner never plugged");
                drop(refused);
                return;
            }
        }
        self.arrived.notify_waiters();
    }

    /// The service is plugging this VM into `network` with `address`: the
    /// profile decides once, the network's cable is brought up in the guest,
    /// and an allowed plug waits under `token` for the service to present it.
    pub(crate) async fn expect(
        &self,
        token: &str,
        network: NetworkIdentity,
        address: Ipv4Addr,
        prefix: u8,
    ) -> Result<()> {
        let token = parse_token(token)?;
        let network_id = network.id.to_string();
        let audit = self.publisher.private_link_audit(network, address)?;
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
        let cable = self.cable_for(&network_id);
        self.control
            .send(ServiceToProcess::PlugCable { cable, address, prefix })
            .await
            .context("the guest control channel is gone")?;
        let now = Instant::now();
        let mut pending = self.pending.lock().unwrap();
        pending.retain(|_, plug| plug.expires > now);
        let admitted = pending.len() < MAX_PENDING && !pending.contains_key(&token);
        if admitted {
            pending.insert(
                token,
                PendingPlug {
                    audit,
                    cable,
                    expires: now + TOKEN_LIFETIME,
                },
            );
        }
        drop(pending);
        ensure!(admitted, "too many plugs awaiting the service, or a duplicate token");
        Ok(())
    }

    /// The network's cable, given an id the first time it is plugged.
    fn cable_for(&self, network: &str) -> u32 {
        let mut table = self.table.lock().unwrap();
        if let Some((cable, _)) = table.cables.iter().find(|(_, entry)| entry.network == network) {
            return *cable;
        }
        table.next += 1;
        let cable = table.next;
        table.cables.insert(
            cable,
            Cable {
                network: network.to_string(),
                guest: None,
                epoch: 0,
                held: None,
                released: 0,
            },
        );
        cable
    }

    /// The VM left `network`: its cable is forgotten and taken down in the
    /// guest. Leaving a network the VM has no cable for is already done.
    pub(crate) async fn detach(&self, network: &str) -> Result<()> {
        let removed = self.table.lock().unwrap().remove_network(network);
        let Some((cable, entry)) = removed else {
            return Ok(());
        };
        drop(entry);
        self.arrived.notify_waiters();
        self.control
            .send(ServiceToProcess::UnplugCable { cable })
            .await
            .context("the guest control channel is gone")
    }

    fn redeem(&self, token: u64) -> Option<PendingPlug> {
        let plug = self.pending.lock().unwrap().remove(&token)?;
        (plug.expires > Instant::now()).then_some(plug)
    }

    /// Claim `cable`'s stream once it is free and newer than the last one a
    /// port let go of, within `deadline`.
    async fn claim(&self, cable: u32, deadline: Duration) -> Result<(std::os::fd::OwnedFd, u64)> {
        let wait = async {
            loop {
                let arrived = self.arrived.notified();
                let claimed = self.table.lock().unwrap().claim(cable)?;
                if let Some(claimed) = claimed {
                    return Ok(claimed);
                }
                arrived.await;
            }
        };
        tokio::time::timeout(deadline, wait)
            .await
            .context("the guest never connected a fresh stream for the cable")?
    }

    /// The service presented `token` on `socket`: answer with the cable's
    /// guest stream and hold both for as long as the service holds the socket.
    pub(crate) async fn take(&self, token: u64, socket: std::os::unix::net::UnixStream) -> Result<()> {
        self.take_within(token, socket, GUEST_DEADLINE).await
    }

    async fn take_within(&self, token: u64, socket: std::os::unix::net::UnixStream, deadline: Duration) -> Result<()> {
        let plug = self.redeem(token).context("unknown, reused or expired plug token")?;
        let (stream, epoch) = self.claim(plug.cable, deadline).await?;
        // From here every way out -- release, error or a dropped future -- lets
        // go of the stream, or the cable could never be plugged again.
        let held = Held {
            cables: self,
            cable: plug.cable,
            epoch,
        };
        let sender = Sender::new(socket.try_clone()?)?;
        sender
            .send(&seat_frame(SEAT_LINK, token), &[stream.as_raw_fd()])
            .await
            .context("answer the service with the guest stream")?;
        drop(stream);
        plug.audit
            .record(
                RuntimeSecurityEventType::NetworkConnectResult,
                NetworkReason::Connected,
                0,
                0,
            )
            .await?;
        tracing::info!(cable = plug.cable, "network cable granted to the switch");
        // The service's close is the signal that the port is over; a socket
        // that fails is as closed as one that ends.
        let mut watch = tokio::net::UnixStream::from_std(socket)?;
        let mut sink = [0u8; 64];
        while matches!(watch.read(&mut sink).await, Ok(read) if read != 0) {}
        drop(held);
        plug.audit
            .record(RuntimeSecurityEventType::NetworkClose, NetworkReason::Complete, 0, 0)
            .await?;
        tracing::info!(cable = plug.cable, "network cable released; its guest stream ends");
        Ok(())
    }
}

/// A port holding `cable`'s stream of `epoch`. Dropping it lets go: the
/// stream ends so the pump reconnects, and the next plug waits for that.
struct Held<'a> {
    cables: &'a Cables,
    cable: u32,
    epoch: u64,
}

impl Drop for Held<'_> {
    fn drop(&mut self) {
        let stream = {
            let mut table = self.cables.table.lock().unwrap();
            table.cables.get_mut(&self.cable).and_then(|entry| {
                entry.released = self.epoch;
                entry.held = None;
                (entry.epoch == self.epoch).then(|| entry.guest.take()).flatten()
            })
        };
        drop(stream);
        self.cables.arrived.notify_waiters();
    }
}

pub(crate) fn parse_token(text: &str) -> Result<u64> {
    ensure!(text.len() == 16, "link token must be sixteen hex digits");
    u64::from_str_radix(text, 16).context("link token is not hex")
}

#[cfg(test)]
mod tests;
