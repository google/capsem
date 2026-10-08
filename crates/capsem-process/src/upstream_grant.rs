//! Process-side client for coordinator-minted upstream descriptors.

use std::io;
use std::os::fd::{AsFd as _, BorrowedFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use anyhow::{anyhow, Result};
use capsem_core::net::dns::{DnsDatagram, DnsGrantFuture, DnsUpstreamGrants};
use capsem_foundation::unix::fd::{self, SocketShutdown};
use capsem_foundation::unix::router_channel::{DescriptorReceiver, DescriptorSender};
use capsem_proto::upstream_grant::{
    decode_upstream_grant_response, encode_upstream_grant_request, UpstreamDescriptorKind, UpstreamGrantRequest,
    UpstreamGrantResponse, UPSTREAM_GRANT_FRAME_SIZE, UPSTREAM_GRANT_MAX_FDS,
};
use tokio::sync::{mpsc, oneshot};

const COMMAND_CAPACITY: usize = 64;
const WIRE_TIMEOUT: Duration = Duration::from_secs(2);

type WireSender = DescriptorSender<UPSTREAM_GRANT_FRAME_SIZE, UPSTREAM_GRANT_MAX_FDS>;
type WireReceiver = DescriptorReceiver<UPSTREAM_GRANT_FRAME_SIZE, UPSTREAM_GRANT_MAX_FDS>;

struct OpenDns {
    upstream_index: u16,
    policy_digest: String,
    reply: oneshot::Sender<Result<DnsDatagram, String>>,
}

/// Serializes grant protocol traffic over the inherited generation channel.
#[derive(Clone)]
pub(crate) struct UpstreamGrantClient {
    opens: mpsc::Sender<OpenDns>,
    policy_digest: Arc<RwLock<String>>,
}

impl UpstreamGrantClient {
    pub(crate) fn start(socket: UnixStream, policy_digest: String) -> io::Result<Self> {
        let sender = WireSender::new(socket.try_clone()?)?;
        let receiver = WireReceiver::new(socket.try_clone()?)?;
        let (opens_tx, opens) = mpsc::channel(COMMAND_CAPACITY);
        let (actor_releases, releases) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let result = run_actor(sender, receiver, opens, releases, actor_releases).await;
            if let Err(error) = result {
                tracing::warn!(%error, "upstream grant channel stopped");
            }
            if let Err(error) = fd::shutdown(socket.as_fd(), SocketShutdown::Both) {
                tracing::debug!(%error, "failed to shut down upstream grant channel");
            }
        });
        Ok(Self {
            opens: opens_tx,
            policy_digest: Arc::new(RwLock::new(policy_digest)),
        })
    }

    pub(crate) fn replace_policy_digest(&self, policy_digest: String) {
        *self.policy_digest.write().unwrap() = policy_digest;
    }

    #[cfg(test)]
    pub(crate) fn policy_digest(&self) -> String {
        self.policy_digest.read().unwrap().clone()
    }

    #[cfg(test)]
    pub(crate) fn test_handle(policy_digest: String) -> Self {
        let (opens, _commands) = mpsc::channel(COMMAND_CAPACITY);
        Self {
            opens,
            policy_digest: Arc::new(RwLock::new(policy_digest)),
        }
    }
}

impl DnsUpstreamGrants for UpstreamGrantClient {
    fn open(&self, upstream_index: u16) -> DnsGrantFuture<'_> {
        Box::pin(async move {
            let policy_digest = self.policy_digest.read().unwrap().clone();
            let (reply, result) = oneshot::channel();
            self.opens
                .send(OpenDns {
                    upstream_index,
                    policy_digest,
                    reply,
                })
                .await
                .map_err(|_| anyhow!("upstream grant channel is closed"))?;
            result
                .await
                .map_err(|_| anyhow!("upstream grant channel stopped"))?
                .map_err(anyhow::Error::msg)
        })
    }
}

async fn run_actor(
    sender: WireSender,
    receiver: WireReceiver,
    mut opens: mpsc::Receiver<OpenDns>,
    mut releases: mpsc::UnboundedReceiver<u64>,
    release_sender: mpsc::UnboundedSender<u64>,
) -> Result<(), String> {
    let mut next_request_id = 1_u64;
    loop {
        tokio::select! {
            biased;
            release = releases.recv() => {
                let Some(resource_id) = release else {
                    if opens.is_closed() {
                        return Ok(());
                    }
                    continue;
                };
                send_request(&sender, &UpstreamGrantRequest::Release { resource_id }).await?;
            }
            open = opens.recv() => {
                let Some(open) = open else {
                    return Ok(());
                };
                let request_id = next_request_id;
                next_request_id = next_request_id.checked_add(1).ok_or("upstream request id exhausted")?;
                match open_dns(&sender, &receiver, &release_sender, request_id, &open).await {
                    Ok(result) => {
                        let _ = open.reply.send(result);
                    }
                    Err(error) => {
                        let _ = open.reply.send(Err(error.clone()));
                        return Err(error);
                    }
                }
            }
        }
    }
}

async fn open_dns(
    sender: &WireSender,
    receiver: &WireReceiver,
    releases: &mpsc::UnboundedSender<u64>,
    request_id: u64,
    open: &OpenDns,
) -> Result<Result<DnsDatagram, String>, String> {
    send_request(
        sender,
        &UpstreamGrantRequest::OpenDns {
            request_id,
            upstream_index: open.upstream_index,
        },
    )
    .await?;
    let frame = tokio::time::timeout(WIRE_TIMEOUT, receiver.recv())
        .await
        .map_err(|_| "receive upstream grant response timed out".to_string())?
        .map_err(|error| format!("receive upstream grant response: {error}"))?;
    let response = decode_upstream_grant_response(&frame.bytes)
        .map_err(|error| format!("decode upstream grant response: {error:#}"))?;
    if frame.fds.len() != response.expected_descriptor_count() {
        return Err(format!(
            "upstream grant response carried {} descriptors, expected {}",
            frame.fds.len(),
            response.expected_descriptor_count()
        ));
    }
    match response {
        UpstreamGrantResponse::Denied {
            request_id: response_id,
            reason,
        } if response_id == request_id => Ok(Err(format!("upstream DNS grant denied: {reason:?}"))),
        UpstreamGrantResponse::DescriptorGranted {
            request_id: response_id,
            grant_id,
            kind: UpstreamDescriptorKind::DnsUdp,
            policy_digest,
        } if response_id == request_id => {
            let descriptor = frame.fds.into_iter().next().ok_or("DNS grant omitted its descriptor")?;
            send_request(sender, &UpstreamGrantRequest::Adopted { grant_id }).await?;
            if policy_digest != open.policy_digest {
                send_request(sender, &UpstreamGrantRequest::Release { resource_id: grant_id }).await?;
                return Ok(Err(format!(
                    "upstream DNS grant policy mismatch: expected {}, received {policy_digest}",
                    open.policy_digest
                )));
            }
            let socket = datagram_from_descriptor(descriptor)?;
            let releases = releases.clone();
            Ok(Ok(DnsDatagram::new(socket, move || {
                let _ = releases.send(grant_id);
            })))
        }
        response => Err(format!(
            "unexpected upstream DNS grant response for request {request_id}: {response:?}"
        )),
    }
}

async fn send_request(sender: &WireSender, request: &UpstreamGrantRequest) -> Result<(), String> {
    let bytes =
        encode_upstream_grant_request(request).map_err(|error| format!("encode upstream grant request: {error:#}"))?;
    tokio::time::timeout(WIRE_TIMEOUT, sender.send(&bytes, &[]))
        .await
        .map_err(|_| "send upstream grant request timed out".to_string())?
        .map_err(|error| format!("send upstream grant request: {error}"))?;
    Ok(())
}

fn datagram_from_descriptor(descriptor: OwnedFd) -> Result<tokio::net::UdpSocket, String> {
    let socket = std::net::UdpSocket::from(descriptor);
    socket
        .set_nonblocking(true)
        .map_err(|error| format!("make granted DNS socket nonblocking: {error}"))?;
    tokio::net::UdpSocket::from_std(socket).map_err(|error| format!("adopt granted DNS socket: {error}"))
}

/// Duplicate the inherited generation channel before runtime threads exist.
pub(crate) fn adopt_inherited(descriptor: BorrowedFd<'_>) -> io::Result<UnixStream> {
    fd::set_close_on_exec(descriptor)?;
    let owned = fd::duplicate(descriptor)?;
    let socket = UnixStream::from(owned);
    socket.peer_addr()?;
    Ok(socket)
}

#[cfg(test)]
mod tests;
