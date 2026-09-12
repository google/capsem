//! Private datagram flows between members, from this owner's two seats.
//!
//! The guest's tun0 frames arrive on VSOCK 5009. As the **source**, this
//! owner parses each packet's headers, asks the service once per flow, and
//! on a grant relays the flow's packets to the destination owner's relay
//! socket under a one-time token; replies come back on the same stream and
//! go to the guest. As the **destination**, the service tells this owner
//! (`PrivateAccept` with a datagram protocol) which flow to expect; the first
//! frame on its relay socket is the token, the flow is evaluated once against
//! this VM's rules, and from then on validated frames go into the guest's
//! tun0 stream while the guest's matching replies go back.
//!
//! No payload is read on the host: `capsem_network::relay` sees headers.
use anyhow::{bail, ensure, Context, Result};
use capsem_core::container::publish::{AuditFlow, Publisher};
use capsem_core::security_engine::network::{NetworkIdentity, NetworkProtocol, NetworkReason, NetworkVm};
use capsem_core::security_engine::{RuntimeSecurityEventType, SecurityEnforcementAction};
use capsem_core::VsockConnection;
use capsem_network::frames::{read_frame, write_frame};
use capsem_network::relay::{parse, Datagram, FlowKey, Kind, Outbound, Protocol, Relay};
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, Notify};
use tracing::{debug, info, warn};

/// How long a token waits for its stream: the setup deadline.
const TOKEN_LIFETIME: Duration = Duration::from_secs(8);
const MAX_PENDING: usize = 64;
/// Frames queued towards one peer before the guest's next packet is dropped.
const PEER_QUEUE: usize = 64;
/// Frames queued towards the guest.
const GUEST_QUEUE: usize = 256;
const EXPIRY_TICK: Duration = Duration::from_secs(5);
const TOKEN_BYTES: usize = 8;

struct PendingFlow {
    network: NetworkIdentity,
    source: NetworkVm,
    source_address: Ipv4Addr,
    source_port: u16,
    port: u16,
    protocol: NetworkProtocol,
    expires: Instant,
}

/// One relay stream: frames towards it, a way to end it, what it moved.
struct Peer {
    frames: mpsc::Sender<Vec<u8>>,
    stop: Arc<Notify>,
    sent: Arc<AtomicU64>,
}

pub(crate) struct PrivateRelay {
    socket_path: PathBuf,
    engine: Mutex<Relay>,
    pending: Mutex<HashMap<u64, PendingFlow>>,
    peers: Mutex<HashMap<FlowKey, Peer>>,
    guest: Mutex<Option<mpsc::Sender<Vec<u8>>>>,
    publisher: Arc<Publisher>,
    service_socket: PathBuf,
    owner_secret: String,
    vm_id: String,
}

impl PrivateRelay {
    pub(crate) fn new(
        socket_path: PathBuf,
        own: Ipv4Addr,
        publisher: Arc<Publisher>,
        service_socket: PathBuf,
        owner_secret: String,
        vm_id: String,
    ) -> Self {
        Self {
            socket_path,
            engine: Mutex::new(Relay::new(own)),
            pending: Mutex::new(HashMap::new()),
            peers: Mutex::new(HashMap::new()),
            guest: Mutex::new(None),
            publisher,
            service_socket,
            owner_secret,
            vm_id,
        }
    }

    pub(crate) fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// The service admitted a flow towards this VM: remember the token.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn expect(
        &self,
        token: &str,
        network: NetworkIdentity,
        source: NetworkVm,
        source_address: Ipv4Addr,
        source_port: u16,
        port: u16,
        protocol: NetworkProtocol,
    ) -> Result<()> {
        ensure!(
            matches!(protocol, NetworkProtocol::Udp | NetworkProtocol::Icmp),
            "the relay carries udp and icmp echo, not {protocol:?}"
        );
        let token = parse_token(token)?;
        let now = Instant::now();
        let flow = PendingFlow {
            network,
            source,
            source_address,
            source_port,
            port,
            protocol,
            expires: now + TOKEN_LIFETIME,
        };
        let mut pending = self.pending.lock().unwrap();
        pending.retain(|_, flow| flow.expires > now);
        ensure!(
            pending.len() < MAX_PENDING,
            "too many datagram flows awaiting their stream"
        );
        ensure!(!pending.contains_key(&token), "duplicate datagram flow token");
        pending.insert(token, flow);
        drop(pending);
        Ok(())
    }

    fn redeem(&self, token: u64) -> Option<PendingFlow> {
        let now = Instant::now();
        let flow = self.pending.lock().unwrap().remove(&token)?;
        (flow.expires > now).then_some(flow)
    }

    /// Take streams other owners relay, and expire idle flows, for as long
    /// as the socket lives.
    pub(crate) async fn serve(self: Arc<Self>, listener: UnixListener) {
        let ticker = Arc::clone(&self);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(EXPIRY_TICK);
            loop {
                tick.tick().await;
                let ended = ticker.engine.lock().unwrap().expire(Instant::now());
                for (key, report) in ended {
                    debug!(?key, ?report, "private datagram flow idle");
                    ticker.stop_peer(&key);
                }
            }
        });
        loop {
            let stream = match listener.accept().await {
                Ok((stream, _)) => stream,
                Err(error) => {
                    warn!(%error, "private relay socket failed");
                    return;
                }
            };
            let owner = Arc::clone(&self);
            tokio::spawn(async move {
                if let Err(error) = owner.take(stream).await {
                    debug!(%error, "private relay stream refused");
                }
            });
        }
    }

    /// The destination seat: a token, then the flow's frames.
    async fn take(self: &Arc<Self>, mut stream: UnixStream) -> Result<()> {
        let mut first = Vec::new();
        let length = tokio::time::timeout(Duration::from_secs(2), read_frame(&mut stream, &mut first))
            .await
            .context("relay token timed out")??
            .context("relay stream ended before its token")?;
        ensure!(length == TOKEN_BYTES, "relay token frame of {length} bytes");
        let token = u64::from_be_bytes(first[..TOKEN_BYTES].try_into().unwrap());
        let flow = self
            .redeem(token)
            .context("unknown, reused or expired datagram flow token")?;
        let (peer_port, port) = if flow.protocol == NetworkProtocol::Icmp {
            (0, 0)
        } else {
            (flow.source_port, flow.port)
        };
        let audit = self.publisher.private_audit(
            flow.network,
            flow.source,
            (flow.source_address, peer_port).into(),
            port,
            flow.protocol,
        )?;
        let action = audit.authorize().await?;
        if action != SecurityEnforcementAction::Allow {
            let reason = match action {
                SecurityEnforcementAction::Ask => NetworkReason::ApprovalRequired,
                _ => NetworkReason::Blocked,
            };
            audit
                .record(RuntimeSecurityEventType::NetworkConnectResult, reason, 0, 0)
                .await?;
            bail!("datagram flow security decision: {action:?}");
        }
        let key = FlowKey {
            peer: flow.source_address,
            protocol: match flow.protocol {
                NetworkProtocol::Icmp => Protocol::Icmp {
                    identifier: flow.source_port,
                },
                _ => Protocol::Udp {
                    peer_port: flow.source_port,
                },
            },
        };
        self.engine
            .lock()
            .unwrap()
            .expect(key, Instant::now())
            .map_err(|reason| anyhow::anyhow!("datagram flow refused: {reason:?}"))?;
        audit
            .record(
                RuntimeSecurityEventType::NetworkConnectResult,
                NetworkReason::Connected,
                0,
                0,
            )
            .await?;
        info!(?key, "private datagram flow accepted");
        self.run_peer(key, stream, Some(audit));
        Ok(())
    }

    /// Own one relay stream for `key`: frames from it go to the guest after
    /// validation, frames for it come from the guest. Ending the stream, the
    /// idle expiry or a replacement ends the flow and its audit.
    fn run_peer(self: &Arc<Self>, key: FlowKey, stream: UnixStream, audit: Option<AuditFlow>) {
        let (frames, mut queue) = mpsc::channel::<Vec<u8>>(PEER_QUEUE);
        let stop = Arc::new(Notify::new());
        let sent = Arc::new(AtomicU64::new(0));
        let mut received = 0u64;
        let peer = Peer {
            frames,
            stop: Arc::clone(&stop),
            sent: Arc::clone(&sent),
        };
        if let Some(previous) = self.peers.lock().unwrap().insert(key, peer) {
            previous.stop.notify_one();
        }
        let (mut reader, mut writer) = stream.into_split();
        let owner = Arc::clone(self);
        tokio::spawn(async move {
            while let Some(frame) = queue.recv().await {
                if write_frame(&mut writer, &frame).await.is_err() {
                    break;
                }
                sent.fetch_add(frame.len() as u64, Ordering::Relaxed);
            }
            let _ = tokio::io::AsyncWriteExt::shutdown(&mut writer).await;
        });
        tokio::spawn(async move {
            let mut packet = Vec::new();
            loop {
                let next = tokio::select! {
                    _ = stop.notified() => None,
                    next = read_frame(&mut reader, &mut packet) => next.ok().flatten(),
                };
                let Some(length) = next else { break };
                let verdict = owner
                    .engine
                    .lock()
                    .unwrap()
                    .inbound(Instant::now(), key, &packet[..length]);
                match verdict {
                    Ok(()) => {
                        received += length as u64;
                        owner.to_guest(packet[..length].to_vec());
                    }
                    Err(reason) => debug!(?key, ?reason, "private datagram frame dropped"),
                }
            }
            owner.end_flow(key, audit, received).await;
        });
    }

    fn stop_peer(&self, key: &FlowKey) {
        if let Some(peer) = self.peers.lock().unwrap().remove(key) {
            peer.stop.notify_one();
        }
    }

    async fn end_flow(&self, key: FlowKey, audit: Option<AuditFlow>, received: u64) {
        let report = self.engine.lock().unwrap().close(&key);
        let sent = self
            .peers
            .lock()
            .unwrap()
            .remove(&key)
            .map(|peer| peer.sent.load(Ordering::Relaxed))
            .unwrap_or(0);
        debug!(?key, ?report, sent, received, "private datagram flow ended");
        if let Some(audit) = audit {
            if let Err(error) = audit
                .record(
                    RuntimeSecurityEventType::NetworkClose,
                    NetworkReason::Complete,
                    sent,
                    received,
                )
                .await
            {
                warn!(%error, "private datagram close audit failed");
            }
        }
    }

    fn to_guest(&self, packet: Vec<u8>) {
        let guest = self.guest.lock().unwrap().clone();
        match guest {
            Some(guest) => {
                if guest.try_send(packet).is_err() {
                    debug!("guest tun0 stream is not keeping up; datagram dropped");
                }
            }
            None => debug!("no guest tun0 stream; datagram dropped"),
        }
    }

    /// The guest's tun0 stream: its packets feed the source seat, and what
    /// the peers deliver is written back to it. A new stream (the pump was
    /// restarted) replaces the old one.
    pub(crate) fn attach_guest(self: &Arc<Self>, conn: VsockConnection) {
        let stream = conn.try_clone_fd().and_then(|fd| {
            capsem_foundation::unix::fd::set_nonblocking(fd.as_fd(), true)?;
            UnixStream::from_std(std::os::unix::net::UnixStream::from(fd))
        });
        let stream = match stream {
            Ok(stream) => stream,
            Err(error) => {
                warn!(%error, "guest tun0 stream descriptor unavailable");
                return;
            }
        };
        let (mut reader, mut writer) = stream.into_split();
        let (frames, mut queue) = mpsc::channel::<Vec<u8>>(GUEST_QUEUE);
        *self.guest.lock().unwrap() = Some(frames.clone());
        tokio::spawn(async move {
            while let Some(frame) = queue.recv().await {
                if write_frame(&mut writer, &frame).await.is_err() {
                    break;
                }
            }
        });
        let owner = Arc::clone(self);
        tokio::spawn(async move {
            let mut packet = Vec::new();
            let mut frames_in = 0u64;
            loop {
                match read_frame(&mut reader, &mut packet).await {
                    Ok(Some(length)) => {
                        frames_in += 1;
                        owner.from_guest(packet[..length].to_vec());
                    }
                    Ok(None) => break,
                    Err(error) => {
                        warn!(%error, "guest tun0 stream failed");
                        break;
                    }
                }
            }
            let mut guest = owner.guest.lock().unwrap();
            if guest.as_ref().is_some_and(|current| current.same_channel(&frames)) {
                *guest = None;
            }
            drop(guest);
            let counters = owner.engine.lock().unwrap().counters();
            info!(frames_in, ?counters, "guest tun0 stream ended");
            drop(conn);
        });
    }

    fn from_guest(self: &Arc<Self>, packet: Vec<u8>) {
        let outcome = self.engine.lock().unwrap().outbound(Instant::now(), &packet);
        match outcome {
            Outbound::Forward(key) => {
                let peers = self.peers.lock().unwrap();
                match peers.get(&key) {
                    Some(peer) => {
                        if peer.frames.try_send(packet).is_err() {
                            debug!(?key, "relay stream is not keeping up; datagram dropped");
                        }
                    }
                    None => debug!(?key, "admitted flow has no relay stream; datagram dropped"),
                }
            }
            Outbound::Ask(key) => {
                if let Ok(datagram) = parse(&packet) {
                    let owner = Arc::clone(self);
                    tokio::spawn(async move {
                        if let Err(error) = owner.ask(key, datagram).await {
                            info!(?key, %error, "private datagram flow refused");
                            owner.engine.lock().unwrap().refused(key, Instant::now());
                        }
                    });
                }
            }
            Outbound::Held(_) => {}
            Outbound::Dropped(reason) => debug!(?reason, "guest datagram dropped"),
        }
    }

    /// The source seat: ask the service for the flow and, granted, open the
    /// relay stream to the destination owner and send what was held.
    async fn ask(self: &Arc<Self>, key: FlowKey, datagram: Datagram) -> Result<()> {
        let (protocol, port, source_port) = match datagram.kind {
            Kind::Udp {
                source_port,
                destination_port,
            } => ("udp", destination_port, source_port),
            Kind::EchoRequest { identifier } | Kind::EchoReply { identifier } => ("icmp", 0, identifier),
        };
        let request = serde_json::json!({
            "source_vm": self.vm_id,
            "owner_secret": self.owner_secret,
            "source_generation": self.publisher.generation().get(),
            "protocol": protocol,
            "destination": key.peer.to_string(),
            "port": port,
            "source_port": source_port,
        });
        let (status, answer) =
            capsem_core::service_uds::post_json(&self.service_socket, "/networks/private/datagram", &request).await?;
        if status != 200 {
            bail!(
                "service refused ({status}): {}",
                answer["message"].as_str().unwrap_or(&answer.to_string())
            );
        }
        let relay_socket = answer["relay_socket"]
            .as_str()
            .context("grant without a relay socket")?
            .to_string();
        let token = parse_token(answer["token"].as_str().context("grant without a token")?)?;
        let mut stream = UnixStream::connect(&relay_socket)
            .await
            .with_context(|| format!("connect destination owner at {relay_socket}"))?;
        write_frame(&mut stream, &token.to_be_bytes())
            .await
            .context("send the relay token")?;
        self.run_peer(key, stream, None);
        let held = self.engine.lock().unwrap().admitted(key, Instant::now());
        let peers = self.peers.lock().unwrap();
        if let Some(peer) = peers.get(&key) {
            for packet in held {
                let _ = peer.frames.try_send(packet);
            }
        }
        drop(peers);
        info!(
            ?key,
            destination_vm = answer["destination_vm"].as_str().unwrap_or(""),
            "private datagram flow relayed"
        );
        Ok(())
    }
}

fn parse_token(text: &str) -> Result<u64> {
    ensure!(text.len() == 16, "datagram flow token must be sixteen hex digits");
    u64::from_str_radix(text, 16).context("datagram flow token is not hex")
}

/// The protocol a `PrivateAccept` names, as the audit facts want it.
pub(crate) fn parse_protocol(name: &str) -> Result<NetworkProtocol> {
    match name {
        "udp" => Ok(NetworkProtocol::Udp),
        "icmp" => Ok(NetworkProtocol::Icmp),
        other => bail!("unknown datagram protocol {other:?}"),
    }
}

#[cfg(test)]
mod tests;
