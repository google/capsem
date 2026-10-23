//! A descriptor capability holds one original caller, with explicit cancellation.
use std::collections::HashMap;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use capsem_foundation::ipc_channel::{self, Receiver, Sender};
use capsem_proto::approval::{
    ApprovalClientMessage, ApprovalOutcome, ApprovalRegistration, ApprovalRequest, ApprovalServerMessage,
};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

enum Command {
    Begin {
        id: u64,
        request: ApprovalRequest,
        reply: oneshot::Sender<ApprovalOutcome>,
        registered: Option<oneshot::Sender<ApprovalRegistration>>,
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
    pending: Arc<tokio::sync::Semaphore>,
    held_bytes: Arc<tokio::sync::Semaphore>,
}

impl ApprovalCapability {
    #[cfg(test)]
    pub(crate) fn connected(client: ApprovalClient) -> Self {
        let (_, receiver) = tokio::sync::watch::channel(Some(Ok(client)));
        Self::with_setup(receiver, tokio::spawn(async {}))
    }

    pub fn new(upstream: Arc<crate::net::upstream_grant::UpstreamGrantClient>) -> Self {
        let (sender, client) = tokio::sync::watch::channel(None);
        let setup = tokio::spawn(async move {
            let result = match upstream.open_approval().await {
                Ok(stream) => ApprovalClient::from_stream(stream).map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            sender.send_replace(Some(result));
        });
        Self::with_setup(client, setup)
    }

    fn with_setup(
        client: tokio::sync::watch::Receiver<Option<Result<ApprovalClient, String>>>,
        setup: tokio::task::JoinHandle<()>,
    ) -> Self {
        Self {
            client,
            setup,
            pending: Arc::new(tokio::sync::Semaphore::new(capsem_proto::APPROVAL_PENDING_PER_SESSION)),
            held_bytes: Arc::new(tokio::sync::Semaphore::new(capsem_proto::APPROVAL_HELD_BYTES_TOTAL)),
        }
    }

    pub async fn decide(&self, request: ApprovalRequest) -> ApprovalOutcome {
        self.decide_registered(request, None).await
    }

    pub(crate) async fn decide_registered(
        &self,
        request: ApprovalRequest,
        registered: Option<oneshot::Sender<ApprovalRegistration>>,
    ) -> ApprovalOutcome {
        if request.validate().is_err() {
            return ApprovalOutcome::Unavailable;
        }
        // Reserve before waiting for descriptor setup: each caller retains
        // its original body while even an unavailable coordinator is pending.
        let Ok(_pending) = Arc::clone(&self.pending).try_acquire_owned() else {
            return ApprovalOutcome::Unavailable;
        };
        let Ok(bytes) = u32::try_from(request.held_bytes) else {
            return ApprovalOutcome::Unavailable;
        };
        let Ok(_bytes) = Arc::clone(&self.held_bytes).try_acquire_many_owned(bytes) else {
            return ApprovalOutcome::Unavailable;
        };
        tokio::time::timeout(deadline(), async {
            let mut receiver = self.client.clone();
            let client = match receiver.wait_for(Option::is_some).await {
                Ok(client) => client.clone(),
                Err(_) => return ApprovalOutcome::Unavailable,
            };
            match client {
                Some(Ok(client)) => client.decide_registered(request, registered).await,
                _ => ApprovalOutcome::Unavailable,
            }
        })
        .await
        .unwrap_or(ApprovalOutcome::Expired)
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
        self.decide_registered(request, None).await
    }

    async fn decide_registered(
        &self,
        request: ApprovalRequest,
        registered: Option<oneshot::Sender<ApprovalRegistration>>,
    ) -> ApprovalOutcome {
        if request.validate().is_err() || self.stopped.is_cancelled() {
            return ApprovalOutcome::Unavailable;
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        if id == 0 {
            self.stopped.cancel();
            return ApprovalOutcome::Unavailable;
        }
        let (reply, result) = oneshot::channel();
        if self
            .commands
            .try_send(Command::Begin {
                id,
                request,
                reply,
                registered,
            })
            .is_err()
        {
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
    struct Waiting {
        request: ApprovalRequest,
        reply: oneshot::Sender<ApprovalOutcome>,
        registered: Option<oneshot::Sender<ApprovalRegistration>>,
        received: bool,
    }
    let mut pending = HashMap::<u64, Waiting>::new();
    loop {
        tokio::select! {
            biased;
            _ = stopped.cancelled() => break,
            message = receive.recv() => match message {
                Ok(ApprovalServerMessage::Resolved {request_id,outcome}) => if let Some(waiter) = pending.remove(&request_id) {
                    let _ = waiter.reply.send(outcome);
                },
                Ok(ApprovalServerMessage::Registered {request_id,registration}) => if let Some(waiter) = pending.get_mut(&request_id) {
                    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
                        .ok().and_then(|elapsed|i64::try_from(elapsed.as_millis()).ok()).unwrap_or(i64::MAX);
                    if waiter.received || registration.validate().is_err()
                        || registration.ask_id != waiter.request.ask_id || registration.event_id != waiter.request.event_id
                        || registration.expires_at_unix_ms <= now
                        || registration.expires_at_unix_ms > now.saturating_add(deadline().as_millis() as i64) {
                        break;
                    }
                    waiter.received = true;
                    if let Some(registered) = waiter.registered.take() { let _ = registered.send(registration); }
                },
                Err(_) => break,
            },
            command = commands.recv() => match command {
                Some(Command::Begin { id, request, reply, registered }) => {
                    if pending.len() >= capsem_proto::APPROVAL_PENDING_PER_SESSION {
                        let _ = reply.send(ApprovalOutcome::Unavailable);
                        continue;
                    }
                    let request_identity = request.clone();
                    if !matches!(tokio::time::timeout(deadline(), send.send(ApprovalClientMessage::Begin { request_id: id, request })).await, Ok(Ok(()))) {
                        break;
                    }
                    pending.insert(id, Waiting {request:request_identity,reply,registered,received:false});
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
