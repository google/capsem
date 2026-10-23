//! A descriptor capability holds one original caller, with explicit cancellation.
use std::collections::HashMap;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use capsem_foundation::ipc_channel::{self, Receiver, Sender};
use capsem_proto::approval::{ApprovalClientMessage, ApprovalOutcome, ApprovalRequest, ApprovalServerMessage};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

enum Command {
    Begin {
        id: u64,
        request: ApprovalRequest,
        reply: oneshot::Sender<ApprovalOutcome>,
    },
    Cancel {
        id: u64,
    },
}

#[derive(Clone)]
pub struct ApprovalClient {
    commands: mpsc::Sender<Command>,
    stopped: CancellationToken,
    next: Arc<AtomicU64>,
}

/// Reuses one descriptor per inherited broker; denied setup is never retried
/// automatically, and no request selects a service address or another VM.
pub struct ApprovalCapability {
    client: tokio::sync::watch::Receiver<Option<Result<ApprovalClient, String>>>,
    setup: tokio::task::JoinHandle<()>,
}

impl ApprovalCapability {
    pub fn new(upstream: Arc<crate::net::upstream_grant::UpstreamGrantClient>) -> Self {
        let (sender, client) = tokio::sync::watch::channel(None);
        let setup = tokio::spawn(async move {
            let result = match upstream.open_approval().await {
                Ok(stream) => ApprovalClient::from_stream(stream).map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            sender.send_replace(Some(result));
        });
        Self { client, setup }
    }

    pub async fn decide(&self, request: ApprovalRequest) -> ApprovalOutcome {
        let mut receiver = self.client.clone();
        let client = match tokio::time::timeout(deadline(), receiver.wait_for(Option::is_some)).await {
            Ok(Ok(client)) => client.clone(),
            _ => return ApprovalOutcome::Unavailable,
        };
        match client {
            Some(Ok(client)) => client.decide(request).await,
            _ => ApprovalOutcome::Unavailable,
        }
    }
}

impl Drop for ApprovalCapability {
    fn drop(&mut self) {
        self.setup.abort();
    }
}

impl ApprovalClient {
    pub fn from_stream(stream: UnixStream) -> std::io::Result<Self> {
        let (send, receive) = ipc_channel::channel_from_std_with_limit(stream, frame_limit())?;
        let (commands, inbox) = mpsc::channel(capsem_proto::APPROVAL_PENDING_PER_SESSION);
        let stopped = CancellationToken::new();
        tokio::spawn(actor(send, receive, inbox, stopped.clone()));
        Ok(Self {
            commands,
            stopped,
            next: Arc::new(AtomicU64::new(1)),
        })
    }

    /// Dropping this future cancels its original operation. A saturated cancel
    /// queue closes the capability, causing all its requests to fail closed.
    pub async fn decide(&self, request: ApprovalRequest) -> ApprovalOutcome {
        if request.validate().is_err() || self.stopped.is_cancelled() {
            return ApprovalOutcome::Unavailable;
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        if id == 0 {
            self.stopped.cancel();
            return ApprovalOutcome::Unavailable;
        }
        let (reply, result) = oneshot::channel();
        if self.commands.try_send(Command::Begin { id, request, reply }).is_err() {
            return ApprovalOutcome::Unavailable;
        }
        let mut guard = Cancellation {
            id,
            client: self,
            armed: true,
        };
        let outcome = tokio::select! {
            biased;
            _ = self.stopped.cancelled() => return ApprovalOutcome::Unavailable,
            result = result => result.unwrap_or(ApprovalOutcome::Unavailable),
            _ = tokio::time::sleep(deadline()) => return ApprovalOutcome::Expired,
        };
        guard.armed = false;
        outcome
    }
}

struct Cancellation<'a> {
    id: u64,
    client: &'a ApprovalClient,
    armed: bool,
}
impl Drop for Cancellation<'_> {
    fn drop(&mut self) {
        if self.armed && self.client.commands.try_send(Command::Cancel { id: self.id }).is_err() {
            self.client.stopped.cancel();
        }
    }
}

pub fn frame_limit() -> u32 {
    (capsem_proto::APPROVAL_CONTEXT_BYTES * 2) as u32
}
fn deadline() -> Duration {
    Duration::from_secs(capsem_proto::APPROVAL_TIMEOUT_SECONDS as u64)
}

async fn actor(
    send: Sender<ApprovalClientMessage>,
    receive: Receiver<ApprovalServerMessage>,
    mut commands: mpsc::Receiver<Command>,
    stopped: CancellationToken,
) {
    let mut pending = HashMap::<u64, oneshot::Sender<ApprovalOutcome>>::new();
    loop {
        tokio::select! {
            biased;
            _ = stopped.cancelled() => break,
            message = receive.recv() => match message {
                Ok(message) => if let Some(reply) = pending.remove(&message.request_id) {
                    let _ = reply.send(message.outcome);
                },
                Err(_) => break,
            },
            command = commands.recv() => match command {
                Some(Command::Begin { id, request, reply }) => {
                    if pending.len() >= capsem_proto::APPROVAL_PENDING_PER_SESSION {
                        let _ = reply.send(ApprovalOutcome::Unavailable);
                        continue;
                    }
                    if !matches!(tokio::time::timeout(deadline(), send.send(ApprovalClientMessage::Begin { request_id: id, request })).await, Ok(Ok(()))) {
                        break;
                    }
                    pending.insert(id, reply);
                }
                Some(Command::Cancel { id }) => {
                    pending.remove(&id);
                    if !matches!(tokio::time::timeout(deadline(), send.send(ApprovalClientMessage::Cancel { request_id: id })).await, Ok(Ok(()))) {
                        break;
                    }
                }
                None => break,
            },
        }
    }
    stopped.cancel();
    // Dropping reply senders fails every still-held caller closed.
}

#[cfg(test)]
mod tests;
