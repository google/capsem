//! Process-side client for coordinator-minted upstream descriptors.

use std::io;
use std::os::fd::{AsFd as _, BorrowedFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use anyhow::{anyhow, Result};
use capsem_core::net::dns::{DnsDatagram, DnsGrantFuture, DnsUpstreamGrants};
use capsem_core::net::mitm_proxy::{
    protocol::Protocol, GrantedTcpStream, TcpConnectGrantFuture, TcpGrantSelection, TcpResolveGrantFuture,
    TcpUpstreamGrants,
};
use capsem_foundation::unix::fd::{self, SocketShutdown};
use capsem_foundation::unix::router_channel::{DescriptorReceiver, DescriptorSender};
use capsem_proto::upstream_grant::{
    decode_upstream_grant_response, encode_upstream_grant_request, UpstreamDescriptorKind, UpstreamGrantRequest,
    UpstreamGrantResponse, UpstreamProtocol, UPSTREAM_GRANT_FRAME_SIZE, UPSTREAM_GRANT_MAX_FDS,
};
use tokio::sync::{mpsc, oneshot};

const COMMAND_CAPACITY: usize = 64;
// The coordinator permits a TCP dial to take ten seconds. Leave room for its
// response framing while still bounding a wedged generation channel.
const WIRE_TIMEOUT: Duration = Duration::from_secs(12);

type WireSender = DescriptorSender<UPSTREAM_GRANT_FRAME_SIZE, UPSTREAM_GRANT_MAX_FDS>;
type WireReceiver = DescriptorReceiver<UPSTREAM_GRANT_FRAME_SIZE, UPSTREAM_GRANT_MAX_FDS>;

enum Command {
    SetGuestMode {
        relative_path: Vec<u8>,
        mode: u16,
        reply: oneshot::Sender<Result<(), String>>,
    },
    OpenDns {
        upstream_index: u16,
        policy_digest: String,
        reply: oneshot::Sender<Result<DnsDatagram, String>>,
    },
    ResolveTcp {
        protocol: Protocol,
        host: String,
        port: u16,
        policy_digest: String,
        reply: oneshot::Sender<Result<TcpGrantSelection, String>>,
    },
    ConnectTcp {
        selection_id: u64,
        policy_digest: String,
        reply: oneshot::Sender<Result<GrantedTcpStream, String>>,
    },
}

/// Serializes grant protocol traffic over the inherited generation channel.
#[derive(Clone)]
pub(crate) struct UpstreamGrantClient {
    commands: mpsc::Sender<Command>,
    policy_digest: Arc<RwLock<String>>,
}

impl UpstreamGrantClient {
    pub(crate) fn start(socket: UnixStream, policy_digest: String) -> io::Result<Self> {
        let sender = WireSender::new(socket.try_clone()?)?;
        let receiver = WireReceiver::new(socket.try_clone()?)?;
        let (commands_tx, commands) = mpsc::channel(COMMAND_CAPACITY);
        let (actor_releases, releases) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let result = run_actor(sender, receiver, commands, releases, actor_releases).await;
            if let Err(error) = result {
                tracing::warn!(%error, "upstream grant channel stopped");
            }
            if let Err(error) = fd::shutdown(socket.as_fd(), SocketShutdown::Both) {
                tracing::debug!(%error, "failed to shut down upstream grant channel");
            }
        });
        Ok(Self {
            commands: commands_tx,
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
        let (commands, _requests) = mpsc::channel(COMMAND_CAPACITY);
        Self {
            commands,
            policy_digest: Arc::new(RwLock::new(policy_digest)),
        }
    }
}

impl capsem_core::GuestMetadataAuthority for UpstreamGrantClient {
    fn set_mode(&self, relative_path: &[u8], mode: u16) -> io::Result<()> {
        let (reply, result) = oneshot::channel();
        self.commands
            .blocking_send(Command::SetGuestMode {
                relative_path: relative_path.to_vec(),
                mode,
                reply,
            })
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "upstream grant channel is closed"))?;
        result
            .blocking_recv()
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "upstream grant channel stopped"))?
            .map_err(io::Error::other)
    }
}

impl DnsUpstreamGrants for UpstreamGrantClient {
    fn open(&self, upstream_index: u16) -> DnsGrantFuture<'_> {
        Box::pin(async move {
            let policy_digest = self.policy_digest.read().unwrap().clone();
            let (reply, result) = oneshot::channel();
            self.commands
                .send(Command::OpenDns {
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

impl TcpUpstreamGrants for UpstreamGrantClient {
    fn resolve(&self, protocol: Protocol, host: &str, port: u16) -> TcpResolveGrantFuture<'_> {
        let host = host.to_owned();
        Box::pin(async move {
            let policy_digest = self.policy_digest.read().unwrap().clone();
            let (reply, result) = oneshot::channel();
            self.commands
                .send(Command::ResolveTcp {
                    protocol,
                    host,
                    port,
                    policy_digest,
                    reply,
                })
                .await
                .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "upstream grant channel is closed"))?;
            result
                .await
                .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "upstream grant channel stopped"))?
                .map_err(io::Error::other)
        })
    }

    fn connect(&self, selection_id: u64) -> TcpConnectGrantFuture<'_> {
        Box::pin(async move {
            let policy_digest = self.policy_digest.read().unwrap().clone();
            let (reply, result) = oneshot::channel();
            self.commands
                .send(Command::ConnectTcp {
                    selection_id,
                    policy_digest,
                    reply,
                })
                .await
                .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "upstream grant channel is closed"))?;
            result
                .await
                .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "upstream grant channel stopped"))?
                .map_err(io::Error::other)
        })
    }
}

async fn run_actor(
    sender: WireSender,
    receiver: WireReceiver,
    mut commands: mpsc::Receiver<Command>,
    mut releases: mpsc::UnboundedReceiver<u64>,
    release_sender: mpsc::UnboundedSender<u64>,
) -> Result<(), String> {
    let mut next_request_id = 1_u64;
    loop {
        tokio::select! {
            biased;
            release = releases.recv() => {
                let Some(resource_id) = release else {
                    if commands.is_closed() {
                        return Ok(());
                    }
                    continue;
                };
                send_request(&sender, &UpstreamGrantRequest::Release { resource_id }).await?;
            }
            command = commands.recv() => {
                let Some(command) = command else {
                    return Ok(());
                };
                let request_id = next_request_id;
                next_request_id = next_request_id.checked_add(1).ok_or("upstream request id exhausted")?;
                dispatch(&sender, &receiver, &release_sender, request_id, command).await?;
            }
        }
    }
}

async fn dispatch(
    sender: &WireSender,
    receiver: &WireReceiver,
    releases: &mpsc::UnboundedSender<u64>,
    request_id: u64,
    command: Command,
) -> Result<(), String> {
    match command {
        Command::SetGuestMode {
            relative_path,
            mode,
            reply,
        } => finish(
            reply,
            set_guest_mode(sender, receiver, request_id, relative_path, mode).await,
        ),
        Command::OpenDns {
            upstream_index,
            policy_digest,
            reply,
        } => finish(
            reply,
            open_dns(sender, receiver, releases, request_id, upstream_index, &policy_digest).await,
        ),
        Command::ResolveTcp {
            protocol,
            host,
            port,
            policy_digest,
            reply,
        } => finish(
            reply,
            resolve_tcp(
                sender,
                receiver,
                releases,
                request_id,
                protocol,
                &host,
                port,
                &policy_digest,
            )
            .await,
        ),
        Command::ConnectTcp {
            selection_id,
            policy_digest,
            reply,
        } => finish(
            reply,
            connect_tcp(sender, receiver, releases, request_id, selection_id, &policy_digest).await,
        ),
    }
}

async fn set_guest_mode(
    sender: &WireSender,
    receiver: &WireReceiver,
    request_id: u64,
    relative_path: Vec<u8>,
    mode: u16,
) -> Result<Result<(), String>, String> {
    send_request(
        sender,
        &UpstreamGrantRequest::SetGuestMode {
            request_id,
            relative_path,
            mode,
        },
    )
    .await?;
    let (response, fds) = receive_response(receiver).await?;
    if !fds.is_empty() {
        return Err("guest mode response carried a descriptor".into());
    }
    match response {
        UpstreamGrantResponse::GuestModeSet {
            request_id: response_id,
        } if response_id == request_id => Ok(Ok(())),
        UpstreamGrantResponse::Denied {
            request_id: response_id,
            reason,
        } if response_id == request_id => Ok(Err(format!("guest mode change denied: {reason:?}"))),
        response => Err(format!(
            "unexpected guest mode response for request {request_id}: {response:?}"
        )),
    }
}

fn finish<T>(
    reply: oneshot::Sender<Result<T, String>>,
    result: Result<Result<T, String>, String>,
) -> Result<(), String> {
    match result {
        Ok(result) => {
            let _ = reply.send(result);
            Ok(())
        }
        Err(error) => {
            let _ = reply.send(Err(error.clone()));
            Err(error)
        }
    }
}

async fn open_dns(
    sender: &WireSender,
    receiver: &WireReceiver,
    releases: &mpsc::UnboundedSender<u64>,
    request_id: u64,
    upstream_index: u16,
    expected_policy_digest: &str,
) -> Result<Result<DnsDatagram, String>, String> {
    send_request(
        sender,
        &UpstreamGrantRequest::OpenDns {
            request_id,
            upstream_index,
        },
    )
    .await?;
    let (response, fds) = receive_response(receiver).await?;
    match response {
        UpstreamGrantResponse::Denied {
            request_id: response_id,
            reason,
        } if response_id == request_id => Ok(Err(format!("upstream DNS grant denied: {reason:?}"))),
        UpstreamGrantResponse::DescriptorGranted {
            request_id: response_id,
            grant_id,
            kind: UpstreamDescriptorKind::DnsUdp,
            policy_digest: response_digest,
        } if response_id == request_id => {
            let descriptor = fds.into_iter().next().ok_or("DNS grant omitted its descriptor")?;
            send_request(sender, &UpstreamGrantRequest::Adopted { grant_id }).await?;
            if response_digest != expected_policy_digest {
                send_request(sender, &UpstreamGrantRequest::Release { resource_id: grant_id }).await?;
                return Ok(Err(format!(
                    "upstream DNS grant policy mismatch: expected {expected_policy_digest}, received {response_digest}"
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

#[allow(clippy::too_many_arguments)]
async fn resolve_tcp(
    sender: &WireSender,
    receiver: &WireReceiver,
    releases: &mpsc::UnboundedSender<u64>,
    request_id: u64,
    protocol: Protocol,
    host: &str,
    port: u16,
    expected_policy_digest: &str,
) -> Result<Result<TcpGrantSelection, String>, String> {
    let protocol = match wire_protocol(protocol) {
        Ok(protocol) => protocol,
        Err(error) => return Ok(Err(error)),
    };
    send_request(
        sender,
        &UpstreamGrantRequest::ResolveTcp {
            request_id,
            protocol,
            host: host.to_owned(),
            port,
        },
    )
    .await?;
    let (response, _fds) = receive_response(receiver).await?;
    match response {
        UpstreamGrantResponse::Denied {
            request_id: response_id,
            reason,
        } if response_id == request_id => Ok(Err(format!("upstream TCP selection denied: {reason:?}"))),
        UpstreamGrantResponse::TcpResolved {
            request_id: response_id,
            selection_id,
            protocol,
            judged_ip,
            policy_digest,
        } if response_id == request_id => {
            if policy_digest != expected_policy_digest {
                send_request(
                    sender,
                    &UpstreamGrantRequest::Release {
                        resource_id: selection_id,
                    },
                )
                .await?;
                return Ok(Err(format!(
                    "upstream TCP selection policy mismatch: expected {expected_policy_digest}, received {policy_digest}"
                )));
            }
            let releases = releases.clone();
            Ok(Ok(TcpGrantSelection::new(
                selection_id,
                core_protocol(protocol),
                judged_ip,
                move || {
                    let _ = releases.send(selection_id);
                },
            )))
        }
        response => Err(format!(
            "unexpected upstream TCP selection response for request {request_id}: {response:?}"
        )),
    }
}

async fn connect_tcp(
    sender: &WireSender,
    receiver: &WireReceiver,
    releases: &mpsc::UnboundedSender<u64>,
    request_id: u64,
    selection_id: u64,
    expected_policy_digest: &str,
) -> Result<Result<GrantedTcpStream, String>, String> {
    send_request(
        sender,
        &UpstreamGrantRequest::ConnectTcp {
            request_id,
            selection_id,
        },
    )
    .await?;
    let (response, fds) = receive_response(receiver).await?;
    match response {
        UpstreamGrantResponse::Denied {
            request_id: response_id,
            reason,
        } if response_id == request_id => Ok(Err(format!("upstream TCP connection denied: {reason:?}"))),
        UpstreamGrantResponse::DescriptorGranted {
            request_id: response_id,
            grant_id,
            kind: UpstreamDescriptorKind::Tcp,
            policy_digest,
        } if response_id == request_id => {
            let descriptor = fds.into_iter().next().ok_or("TCP grant omitted its descriptor")?;
            send_request(sender, &UpstreamGrantRequest::Adopted { grant_id }).await?;
            if policy_digest != expected_policy_digest {
                send_request(sender, &UpstreamGrantRequest::Release { resource_id: grant_id }).await?;
                return Ok(Err(format!(
                    "upstream TCP grant policy mismatch: expected {expected_policy_digest}, received {policy_digest}"
                )));
            }
            let stream = stream_from_descriptor(descriptor)?;
            let releases = releases.clone();
            Ok(Ok(GrantedTcpStream::new(stream, move || {
                let _ = releases.send(grant_id);
            })))
        }
        response => Err(format!(
            "unexpected upstream TCP grant response for request {request_id}: {response:?}"
        )),
    }
}

async fn receive_response(receiver: &WireReceiver) -> Result<(UpstreamGrantResponse, Vec<OwnedFd>), String> {
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
    Ok((response, frame.fds))
}

fn wire_protocol(protocol: Protocol) -> Result<UpstreamProtocol, String> {
    match protocol {
        Protocol::Http => Ok(UpstreamProtocol::Http),
        Protocol::Tls => Ok(UpstreamProtocol::Tls),
        Protocol::McpFrame | Protocol::Unknown => {
            Err(format!("protocol {} cannot select a TCP upstream", protocol.label()))
        }
    }
}

fn core_protocol(protocol: UpstreamProtocol) -> Protocol {
    match protocol {
        UpstreamProtocol::Http => Protocol::Http,
        UpstreamProtocol::Tls => Protocol::Tls,
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

fn stream_from_descriptor(descriptor: OwnedFd) -> Result<tokio::net::TcpStream, String> {
    let stream = std::net::TcpStream::from(descriptor);
    stream
        .set_nonblocking(true)
        .map_err(|error| format!("make granted TCP stream nonblocking: {error}"))?;
    stream
        .set_nodelay(true)
        .map_err(|error| format!("set granted TCP stream nodelay: {error}"))?;
    tokio::net::TcpStream::from_std(stream).map_err(|error| format!("adopt granted TCP stream: {error}"))
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
