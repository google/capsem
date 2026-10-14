use std::io;
use std::net::Ipv4Addr;
use std::os::unix::net::UnixStream;
use std::time::Duration;

use capsem_core::net::dns::private::{Lookup, PrivateNames};
use capsem_foundation::ipc_channel;
use capsem_proto::proxy_control::ProxyChannelCloseReason;
use capsem_proto::proxy_private_names::{ProxyPrivateNameRequest, ProxyPrivateNameResponse};
use tokio::sync::{mpsc, oneshot, watch};

const COMMAND_CAPACITY: usize = 32;
const OPERATION_TIMEOUT: Duration = Duration::from_secs(5);

enum Command {
    AddressOf {
        name: String,
        reply: oneshot::Sender<Option<Ipv4Addr>>,
    },
    NameOf {
        address: Ipv4Addr,
        reply: oneshot::Sender<Option<String>>,
    },
}

impl Command {
    fn fail(self) {
        match self {
            Self::AddressOf { reply, .. } => {
                let _ = reply.send(None);
            }
            Self::NameOf { reply, .. } => {
                let _ = reply.send(None);
            }
        }
    }
}

pub(super) struct PrivateNameClient {
    commands: mpsc::Sender<Command>,
}

impl PrivateNameClient {
    pub(super) fn start(stream: UnixStream) -> io::Result<(Self, watch::Receiver<Option<ProxyChannelCloseReason>>)> {
        let (sender, receiver) =
            ipc_channel::channel_from_std::<ProxyPrivateNameRequest, ProxyPrivateNameResponse>(stream)?;
        let (commands, inbox) = mpsc::channel(COMMAND_CAPACITY);
        let (closed_tx, closed) = watch::channel(None);
        tokio::spawn(async move {
            let reason = run(sender, receiver, inbox).await;
            let _ = closed_tx.send(Some(reason));
        });
        Ok((Self { commands }, closed))
    }
}

impl PrivateNames for PrivateNameClient {
    fn address_of<'a>(&'a self, name: &'a str) -> Lookup<'a, Ipv4Addr> {
        Box::pin(async move {
            let (reply, result) = oneshot::channel();
            self.commands
                .send(Command::AddressOf {
                    name: name.to_string(),
                    reply,
                })
                .await
                .ok()?;
            result.await.ok().flatten()
        })
    }

    fn name_of(&self, address: Ipv4Addr) -> Lookup<'_, String> {
        Box::pin(async move {
            let (reply, result) = oneshot::channel();
            self.commands.send(Command::NameOf { address, reply }).await.ok()?;
            result.await.ok().flatten()
        })
    }
}

async fn run(
    sender: ipc_channel::Sender<ProxyPrivateNameRequest>,
    receiver: ipc_channel::Receiver<ProxyPrivateNameResponse>,
    mut commands: mpsc::Receiver<Command>,
) -> ProxyChannelCloseReason {
    let mut request_id = 1_u64;
    loop {
        tokio::select! {
            unsolicited = receiver.recv() => {
                return match unsolicited {
                    Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => ProxyChannelCloseReason::Disconnected,
                    _ => ProxyChannelCloseReason::ProtocolError,
                };
            }
            command = commands.recv() => {
                let Some(command) = command else {
                    return ProxyChannelCloseReason::Revoked;
                };
                if dispatch(&sender, &receiver, request_id, command).await.is_err() {
                    while let Ok(command) = commands.try_recv() {
                        command.fail();
                    }
                    return ProxyChannelCloseReason::ProtocolError;
                }
                let Some(next) = request_id.checked_add(1) else {
                    return ProxyChannelCloseReason::ProtocolError;
                };
                request_id = next;
            }
        }
    }
}

async fn dispatch(
    sender: &ipc_channel::Sender<ProxyPrivateNameRequest>,
    receiver: &ipc_channel::Receiver<ProxyPrivateNameResponse>,
    request_id: u64,
    command: Command,
) -> Result<(), ()> {
    let request = match &command {
        Command::AddressOf { name, .. } => ProxyPrivateNameRequest::AddressOf {
            request_id,
            name: name.clone(),
        },
        Command::NameOf { address, .. } => ProxyPrivateNameRequest::NameOf {
            request_id,
            address: *address,
        },
    };
    if request.validate().is_err() || sender.send(request).await.is_err() {
        command.fail();
        return Err(());
    }
    let response = match tokio::time::timeout(OPERATION_TIMEOUT, receiver.recv()).await {
        Ok(Ok(response)) if response.request_id() == request_id => response,
        _ => {
            command.fail();
            return Err(());
        }
    };
    match (command, response) {
        (Command::AddressOf { reply, .. }, ProxyPrivateNameResponse::Address { address, .. }) => {
            let _ = reply.send(Some(address));
        }
        (Command::NameOf { reply, .. }, ProxyPrivateNameResponse::Name { name, .. }) => {
            let _ = reply.send(Some(name));
        }
        (Command::AddressOf { reply, .. }, ProxyPrivateNameResponse::NotFound { .. }) => {
            let _ = reply.send(None);
        }
        (Command::NameOf { reply, .. }, ProxyPrivateNameResponse::NotFound { .. }) => {
            let _ = reply.send(None);
        }
        (Command::AddressOf { reply, .. }, ProxyPrivateNameResponse::Rejected { error, .. }) => {
            tracing::warn!(%error, "private address lookup refused");
            let _ = reply.send(None);
        }
        (Command::NameOf { reply, .. }, ProxyPrivateNameResponse::Rejected { error, .. }) => {
            tracing::warn!(%error, "private reverse lookup refused");
            let _ = reply.send(None);
        }
        (command, _) => {
            command.fail();
            return Err(());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
