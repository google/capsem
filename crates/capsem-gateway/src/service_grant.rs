//! Serialized requests for coordinator-minted Unix connection descriptors.

use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::time::Duration;

use capsem_foundation::unix::router_channel::{DescriptorReceiver, DescriptorSender};
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
    Service(oneshot::Sender<io::Result<OwnedFd>>),
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
        let sender = WireSender::new(socket.try_clone()?)?;
        let receiver = WireReceiver::new(socket)?;
        let (commands, requests) = mpsc::channel(COMMAND_CAPACITY);
        tokio::spawn(async move {
            if let Err(error) = run(sender, receiver, requests).await {
                tracing::warn!(%error, "gateway descriptor grant channel stopped");
            }
        });
        Ok(Self { commands })
    }

    pub(crate) async fn open_service(&self) -> io::Result<tokio::net::UnixStream> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::Service(reply))
            .await
            .map_err(|_| closed())?;
        into_tokio(result.await.map_err(|_| closed())??)
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

async fn run(sender: WireSender, receiver: WireReceiver, mut commands: mpsc::Receiver<Command>) -> io::Result<()> {
    let mut request_id = 1_u64;
    while let Some(command) = commands.recv().await {
        let (request, expected, reply) = match command {
            Command::Service(reply) => (
                GatewayGrantRequest::OpenService { request_id },
                GatewayGrantKind::Service,
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
        let result = exchange(&sender, &receiver, request, expected).await;
        let failed = result.is_err();
        let _ = reply.send(result);
        if failed {
            return Err(io::Error::other("gateway descriptor grant exchange failed"));
        }
    }
    Ok(())
}

async fn exchange(
    sender: &WireSender,
    receiver: &WireReceiver,
    request: GatewayGrantRequest,
    expected: GatewayGrantKind,
) -> io::Result<OwnedFd> {
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
            Ok(response.fds.into_iter().next().unwrap())
        }
        GatewayGrantResponse::Denied { reason, .. } if response.fds.is_empty() => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("gateway grant denied: {reason:?}"),
        )),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid gateway descriptor grant response",
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
