//! Private connections between members, from this owner's two seats.
//!
//! As the **destination**, the service tells this owner (`PrivateAccept`)
//! that a connection from a member is admitted under a one-time token; the
//! source owner then delivers the stream on this owner's handoff socket with
//! that token, and the flow joins the publication broker as a private-class
//! pair. As the **source**, a guest connect to a private address arrives on
//! VSOCK 5010; this owner asks the service, and on a grant delivers the
//! stream to the destination owner's socket.
//!
//! The token is redeemed once and expires with the guest setup deadline; a
//! frame with an unknown, reused or expired token is closed without a byte.
use anyhow::{bail, ensure, Context, Result};
use capsem_core::container::publish::{Incoming, Publisher, Source};
use capsem_core::security_engine::network::{NetworkIdentity, NetworkVm};
use capsem_core::VsockConnection;
use capsem_foundation::unix::router_channel::{Frame, Receiver, Sender, FRAME_SIZE};
use capsem_proto::ipc::ServiceToProcess;
use capsem_proto::privatelink::ConnectHeader;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::num::NonZeroU64;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

/// How long a token waits for its stream: the guest setup deadline.
const TOKEN_LIFETIME: Duration = Duration::from_secs(8);
/// Tokens outstanding at once; past this an accept is refused rather than
/// letting a flood of grants pile up unredeemed.
const MAX_PENDING: usize = 64;
const FRAME_VERSION: u8 = 1;
const FRAME_HANDOFF: u8 = 4;

struct PendingAccept {
    network: NetworkIdentity,
    source: NetworkVm,
    source_address: SocketAddr,
    port: u16,
    expires: Instant,
}

pub(crate) struct PrivateHandoff {
    socket_path: PathBuf,
    pending: Mutex<HashMap<u64, PendingAccept>>,
    feed: mpsc::Sender<Incoming>,
    /// Taken by the first delivered stream, which starts the private broker.
    broker_input: Mutex<Option<mpsc::Receiver<Incoming>>>,
    broker_started: tokio::sync::OnceCell<()>,
    publisher: Arc<Publisher>,
    control: mpsc::Sender<ServiceToProcess>,
    service_socket: PathBuf,
    owner_secret: String,
    vm_id: String,
}

impl PrivateHandoff {
    pub(crate) fn new(
        socket_path: PathBuf,
        publisher: Arc<Publisher>,
        control: mpsc::Sender<ServiceToProcess>,
        service_socket: PathBuf,
        owner_secret: String,
        vm_id: String,
    ) -> Self {
        let (feed, broker_input) = mpsc::channel(MAX_PENDING);
        Self {
            socket_path,
            pending: Mutex::new(HashMap::new()),
            feed,
            broker_input: Mutex::new(Some(broker_input)),
            broker_started: tokio::sync::OnceCell::new(),
            publisher,
            control,
            service_socket,
            owner_secret,
            vm_id,
        }
    }

    pub(crate) fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// The service admitted a connection to this VM: remember the token
    /// until its stream arrives or the deadline passes.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn expect(
        &self,
        token: &str,
        network: NetworkIdentity,
        source: NetworkVm,
        source_address: SocketAddr,
        port: u16,
    ) -> Result<()> {
        let token = parse_token(token)?;
        let now = Instant::now();
        let accept = PendingAccept {
            network,
            source,
            source_address,
            port,
            expires: now + TOKEN_LIFETIME,
        };
        let mut pending = self.pending.lock().unwrap();
        pending.retain(|_, accept| accept.expires > now);
        ensure!(
            pending.len() < MAX_PENDING,
            "too many private connections awaiting their stream"
        );
        ensure!(!pending.contains_key(&token), "duplicate private connection token");
        pending.insert(token, accept);
        drop(pending);
        Ok(())
    }

    fn redeem(&self, token: u64) -> Option<PendingAccept> {
        let now = Instant::now();
        let accept = self.pending.lock().unwrap().remove(&token)?;
        (accept.expires > now).then_some(accept)
    }

    /// Take streams other owners deliver, for as long as the socket lives.
    pub(crate) async fn serve(self: Arc<Self>, listener: tokio::net::UnixListener) {
        loop {
            let stream = match listener.accept().await {
                Ok((stream, _)) => stream,
                Err(error) => {
                    warn!(%error, "private handoff socket failed");
                    return;
                }
            };
            let owner = Arc::clone(&self);
            tokio::spawn(async move {
                if let Err(error) = owner.take(stream).await {
                    debug!(%error, "private handoff refused");
                }
            });
        }
    }

    async fn take(self: &Arc<Self>, stream: tokio::net::UnixStream) -> Result<()> {
        let receiver = Receiver::new(stream.into_std()?)?;
        let frame = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .context("handoff frame timed out")??;
        let token = decode_token(&frame)?;
        // The token is spent by the first frame naming it, whatever its
        // shape: one-time means one delivery attempt.
        let accept = self
            .redeem(token)
            .context("unknown, reused or expired private connection token")?;
        let mut fds = frame.fds;
        ensure!(fds.len() == 1, "handoff carried {} descriptors, not one", fds.len());
        let audit = self.publisher.private_audit(
            accept.network,
            accept.source,
            accept.source_address,
            accept.port,
            capsem_core::security_engine::network::NetworkProtocol::Tcp,
        )?;
        self.start_broker().await?;
        let arrival = Incoming {
            source: Source::Stream(fds.pop().unwrap()),
            audit,
            port: accept.port,
            keepalive: Some(receiver),
        };
        self.feed
            .try_send(arrival)
            .map_err(|_| anyhow::anyhow!("private broker is not taking connections"))?;
        Ok(())
    }

    async fn start_broker(self: &Arc<Self>) -> Result<()> {
        self.broker_started
            .get_or_try_init(|| async {
                let input = self
                    .broker_input
                    .lock()
                    .unwrap()
                    .take()
                    .context("private broker input already taken")?;
                self.publisher.serve_private(self.control.clone(), input).await?;
                info!(socket = %self.socket_path.display(), "private connection broker started");
                Ok::<(), anyhow::Error>(())
            })
            .await
            .map(|_| ())
    }

    /// The source seat: a guest connect to `header.destination:port`, asked
    /// of the service and, on a grant, delivered to the destination owner.
    /// Returns once the destination has ended the flow; every failure before
    /// that leaves the guest's connection closed without a byte crossing.
    pub(crate) async fn connect_out(
        &self,
        conn: VsockConnection,
        header: ConnectHeader,
        process_name: String,
    ) -> Result<()> {
        let request = serde_json::json!({
            "source_vm": self.vm_id,
            "owner_secret": self.owner_secret,
            "source_generation": self.publisher.generation().get(),
            "destination": header.destination.to_string(),
            "port": header.port,
            "source_port": header.source_port,
            "process_name": process_name,
        });
        let (status, answer) =
            capsem_core::service_uds::post_json(&self.service_socket, "/networks/private/connect", &request).await?;
        if status != 200 {
            bail!(
                "service refused ({status}): {}",
                answer["message"].as_str().unwrap_or(&answer.to_string())
            );
        }
        let handoff_socket = answer["handoff_socket"]
            .as_str()
            .context("grant without a handoff socket")?
            .to_string();
        let token = parse_token(answer["token"].as_str().context("grant without a token")?)?;
        let destination_vm = answer["destination_vm"].as_str().unwrap_or("").to_string();
        let socket = std::os::unix::net::UnixStream::connect(&handoff_socket)
            .with_context(|| format!("connect destination owner at {handoff_socket}"))?;
        let watch = socket.try_clone()?;
        let sender = Sender::new(socket)?;
        let stream = conn.try_clone_fd()?;
        sender
            .send(&encode_token(token), &[stream.as_raw_fd()])
            .await
            .context("deliver the stream to the destination owner")?;
        drop(stream);
        info!(
            destination = %header.destination, port = header.port, destination_vm, process_name,
            "private connection delivered to the destination owner"
        );
        // The destination keeps the handoff channel open for the life of the
        // flow; its close is the signal to let go of the guest's connection.
        watch.set_nonblocking(true)?;
        let mut watch = tokio::net::UnixStream::from_std(watch)?;
        let mut sink = [0u8; 64];
        while watch.read(&mut sink).await? != 0 {}
        drop(conn);
        Ok(())
    }
}

fn parse_token(text: &str) -> Result<u64> {
    ensure!(text.len() == 16, "private connection token must be sixteen hex digits");
    u64::from_str_radix(text, 16).context("private connection token is not hex")
}

fn encode_token(token: u64) -> [u8; FRAME_SIZE] {
    let mut frame = [0u8; FRAME_SIZE];
    frame[0] = FRAME_VERSION;
    frame[1] = FRAME_HANDOFF;
    frame[2..].copy_from_slice(&token.to_be_bytes());
    frame
}

fn decode_token(frame: &Frame) -> Result<u64> {
    ensure!(
        frame.bytes[0] == FRAME_VERSION,
        "handoff frame version {}",
        frame.bytes[0]
    );
    ensure!(frame.bytes[1] == FRAME_HANDOFF, "handoff frame kind {}", frame.bytes[1]);
    Ok(u64::from_be_bytes(frame.bytes[2..].try_into().unwrap()))
}

/// What the service told us about the source, as the audit facts want it.
pub(crate) fn source_vm(id: String, name: String, generation: u64) -> NetworkVm {
    NetworkVm {
        id,
        name,
        generation: NonZeroU64::new(generation).unwrap_or(NonZeroU64::MIN),
    }
}

#[cfg(test)]
mod tests;
