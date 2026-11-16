//! Serialized requests for coordinator-minted Unix connection descriptors.

use std::io;
use std::os::fd::{AsFd as _, OwnedFd};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use capsem_foundation::unix::router_channel::{DescriptorReceiver, DescriptorSender};
use capsem_proto::gateway_grant::auth::{
    decode_gateway_auth_response, encode_gateway_auth_request, GatewayAuthDenial, GatewayAuthRequest,
    GatewayAuthResponse, GatewayAuthSecret,
};
use capsem_proto::gateway_grant::{
    decode_gateway_grant_response, encode_gateway_grant_request, GatewayGrantKind, GatewayGrantRequest,
    GatewayGrantResponse, GATEWAY_GRANT_FRAME_SIZE, GATEWAY_GRANT_MAX_FDS,
};
use tokio::sync::{mpsc, oneshot};

const COMMAND_CAPACITY: usize = 64;
const WIRE_TIMEOUT: Duration = Duration::from_secs(5);

type WireSender = DescriptorSender<GATEWAY_GRANT_FRAME_SIZE, GATEWAY_GRANT_MAX_FDS>;
type WireReceiver = DescriptorReceiver<GATEWAY_GRANT_FRAME_SIZE, GATEWAY_GRANT_MAX_FDS>;

enum Command {
    AuthenticateAgent {
        access_id: [u8; 16],
        capability: GatewayAuthSecret<32>,
        reply: oneshot::Sender<io::Result<bool>>,
    },
    RegisterOperator {
        token: GatewayAuthSecret<64>,
        reply: oneshot::Sender<io::Result<()>>,
    },
    Service(oneshot::Sender<io::Result<OwnedFd>>),
    Metrics(oneshot::Sender<io::Result<OwnedFd>>),
    Owner {
        vm_id: String,
        reply: oneshot::Sender<io::Result<OwnedFd>>,
    },
}

#[derive(Clone)]
pub(crate) struct GatewayGrantClient {
    commands: mpsc::Sender<Command>,
}

impl GatewayGrantClient {
    pub(crate) fn start(socket: UnixStream) -> io::Result<Self> {
        let peer = capsem_foundation::unix::peer::identity(socket.as_fd())?;
        let sender = WireSender::new(socket.try_clone()?)?;
        let receiver = WireReceiver::new(socket)?;
        let (commands, requests) = mpsc::channel(COMMAND_CAPACITY);
        tokio::spawn(async move {
            if let Err(error) = run(sender, receiver, requests, peer).await {
                tracing::warn!(%error, "gateway descriptor grant channel stopped");
            }
        });
        Ok(Self { commands })
    }

    pub(crate) async fn authenticate_agent(
        &self,
        access_id: [u8; 16],
        capability: GatewayAuthSecret<32>,
    ) -> io::Result<bool> {
        let (reply, result) = oneshot::channel();
        self.commands
            .try_send(Command::AuthenticateAgent {
                access_id,
                capability,
                reply,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => {
                    io::Error::new(io::ErrorKind::WouldBlock, "gateway authentication queue full")
                }
                mpsc::error::TrySendError::Closed(_) => closed(),
            })?;
        tokio::time::timeout(WIRE_TIMEOUT, result)
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "gateway authentication queue timed out"))?
            .map_err(|_| closed())?
    }

    pub(crate) async fn register_operator(&self, token: &str) -> io::Result<()> {
        let bytes = token
            .as_bytes()
            .try_into()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid operator credential length"))?;
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::RegisterOperator {
                token: GatewayAuthSecret::from_bytes(bytes),
                reply,
            })
            .await
            .map_err(|_| closed())?;
        result.await.map_err(|_| closed())?
    }

    pub(crate) async fn open_service(&self) -> io::Result<tokio::net::UnixStream> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::Service(reply))
            .await
            .map_err(|_| closed())?;
        into_tokio(result.await.map_err(|_| closed())??)
    }

    pub(crate) async fn open_metrics(&self) -> io::Result<UnixStream> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::Metrics(reply))
            .await
            .map_err(|_| closed())?;
        Ok(UnixStream::from(result.await.map_err(|_| closed())??))
    }

    pub(crate) async fn open_owner(&self, vm_id: String) -> io::Result<tokio::net::UnixStream> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::Owner { vm_id, reply })
            .await
            .map_err(|_| closed())?;
        into_tokio(result.await.map_err(|_| closed())??)
    }
}

async fn run(
    sender: WireSender,
    receiver: WireReceiver,
    mut commands: mpsc::Receiver<Command>,
    peer: capsem_foundation::unix::peer::PeerIdentity,
) -> io::Result<()> {
    let mut request_id = 1_u64;
    while let Some(command) = commands.recv().await {
        if let Command::AuthenticateAgent {
            access_id,
            capability,
            reply,
        } = command
        {
            let request = GatewayAuthRequest::AuthenticateAgent {
                request_id,
                access_id,
                capability,
            };
            request_id = request_id
                .checked_add(1)
                .ok_or_else(|| io::Error::other("gateway grant request id exhausted"))?;
            let exchanged = exchange_auth(&sender, &receiver, request).await;
            let failed = exchanged.is_err();
            let result = match exchanged {
                Ok(GatewayAuthResponse::AuthenticatedAgent { .. }) => Ok(true),
                Ok(GatewayAuthResponse::Denied {
                    reason: GatewayAuthDenial::Forbidden,
                    ..
                }) => Ok(false),
                Ok(GatewayAuthResponse::Denied {
                    reason: GatewayAuthDenial::Unavailable,
                    ..
                }) => Err(io::Error::other("delegated authority unavailable")),
                Ok(_) => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unexpected delegated authentication response",
                )),
                Err(error) => Err(error),
            };
            let _ = reply.send(result);
            if failed {
                return Err(io::Error::other("gateway authentication exchange failed"));
            }
            continue;
        }
        if let Command::RegisterOperator { token, reply } = command {
            let request = GatewayAuthRequest::RegisterOperator { request_id, token };
            request_id = request_id
                .checked_add(1)
                .ok_or_else(|| io::Error::other("gateway grant request id exhausted"))?;
            let exchanged = exchange_auth(&sender, &receiver, request).await;
            let failed = exchanged.is_err();
            let result = match exchanged {
                Ok(GatewayAuthResponse::RegisteredOperator { .. }) => Ok(()),
                Ok(GatewayAuthResponse::Denied { .. }) => Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "operator bootstrap was refused",
                )),
                Ok(_) => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unexpected operator bootstrap response",
                )),
                Err(error) => Err(error),
            };
            let _ = reply.send(result);
            if failed {
                return Err(io::Error::other("gateway authentication exchange failed"));
            }
            continue;
        }
        let (request, expected, reply) = match command {
            Command::RegisterOperator { .. } | Command::AuthenticateAgent { .. } => {
                return Err(io::Error::other("invalid gateway command dispatch"))
            }
            Command::Service(reply) => (
                GatewayGrantRequest::OpenService { request_id },
                GatewayGrantKind::Service,
                reply,
            ),
            Command::Metrics(reply) => (
                GatewayGrantRequest::OpenMetrics { request_id },
                GatewayGrantKind::Metrics,
                reply,
            ),
            Command::Owner { vm_id, reply } => (
                GatewayGrantRequest::OpenOwnerHandoff { request_id, vm_id },
                GatewayGrantKind::OwnerHandoff,
                reply,
            ),
        };
        request_id = request_id
            .checked_add(1)
            .ok_or_else(|| io::Error::other("gateway grant request id exhausted"))?;
        let exchanged = exchange(&sender, &receiver, request, expected, peer).await;
        let failed = exchanged.is_err()
            || (expected != GatewayGrantKind::Metrics && matches!(exchanged, Ok(GrantResult::Denied(_))));
        let result = match exchanged {
            Ok(GrantResult::Descriptor(fd)) => Ok(fd),
            Ok(GrantResult::Denied(reason)) => Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("gateway grant denied: {reason:?}"),
            )),
            Err(error) => Err(error),
        };
        let _ = reply.send(result);
        if failed {
            return Err(io::Error::other("gateway descriptor grant exchange failed"));
        }
    }
    Ok(())
}

enum GrantResult {
    Descriptor(OwnedFd),
    Denied(capsem_proto::gateway_grant::GatewayGrantDenial),
}

async fn exchange(
    sender: &WireSender,
    receiver: &WireReceiver,
    request: GatewayGrantRequest,
    expected: GatewayGrantKind,
    peer: capsem_foundation::unix::peer::PeerIdentity,
) -> io::Result<GrantResult> {
    let request_id = request.request_id();
    let frame = encode_gateway_grant_request(&request).map_err(io::Error::other)?;
    tokio::time::timeout(WIRE_TIMEOUT, sender.send(&frame, &[]))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "gateway grant request timed out"))??;
    let response = tokio::time::timeout(WIRE_TIMEOUT, receiver.recv())
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "gateway grant response timed out"))??;
    let decoded = decode_gateway_grant_response(&response.bytes).map_err(io::Error::other)?;
    if decoded.request_id() != request_id {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "gateway grant response id mismatch",
        ));
    }
    match decoded {
        GatewayGrantResponse::Granted { kind, .. } if kind == expected && response.fds.len() == 1 => {
            let descriptor = response.fds.into_iter().next().unwrap();
            capsem_foundation::unix::fd::validate_connected_stream(descriptor.as_fd())?;
            if expected == GatewayGrantKind::Metrics {
                capsem_foundation::unix::peer::require(descriptor.as_fd(), peer)?;
            }
            let receipt =
                encode_gateway_grant_request(&GatewayGrantRequest::Adopted { request_id }).map_err(io::Error::other)?;
            tokio::time::timeout(WIRE_TIMEOUT, sender.send(&receipt, &[]))
                .await
                .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "gateway adoption receipt timed out"))??;
            Ok(GrantResult::Descriptor(descriptor))
        }
        GatewayGrantResponse::Denied { reason, .. } if response.fds.is_empty() => Ok(GrantResult::Denied(reason)),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid gateway descriptor grant response",
        )),
    }
}

async fn exchange_auth(
    sender: &WireSender,
    receiver: &WireReceiver,
    request: GatewayAuthRequest,
) -> io::Result<GatewayAuthResponse> {
    let request_id = request.request_id();
    let frame = encode_gateway_auth_request(&request).map_err(io::Error::other)?;
    tokio::time::timeout(WIRE_TIMEOUT, sender.send(&frame, &[]))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "gateway authentication request timed out"))??;
    let response = tokio::time::timeout(WIRE_TIMEOUT, receiver.recv())
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "gateway authentication response timed out"))??;
    if !response.fds.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "gateway authentication response carried descriptors",
        ));
    }
    let response = decode_gateway_auth_response(&response.bytes).map_err(io::Error::other)?;
    if response.request_id() != request_id {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "gateway authentication response identity mismatch",
        ));
    }
    match (&request, response) {
        (_, GatewayAuthResponse::Denied { .. })
        | (GatewayAuthRequest::RegisterOperator { .. }, GatewayAuthResponse::RegisteredOperator { .. }) => Ok(response),
        (
            GatewayAuthRequest::AuthenticateAgent { access_id, .. },
            GatewayAuthResponse::AuthenticatedAgent { access_id: actual, .. },
        ) if *access_id == actual => Ok(response),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "gateway authentication response direction mismatch",
        )),
    }
}

fn into_tokio(descriptor: OwnedFd) -> io::Result<tokio::net::UnixStream> {
    let stream = UnixStream::from(descriptor);
    stream.set_nonblocking(true)?;
    tokio::net::UnixStream::from_std(stream)
}

fn closed() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "gateway descriptor grant channel is closed")
}

#[cfg(test)]
mod tests;
