//! Descriptor-backed writer client for the dedicated ledger owner.

use std::io;
use std::os::unix::net::UnixStream;
use std::sync::{mpsc, Mutex};
use std::time::Duration;

use capsem_foundation::ipc_channel;
use capsem_proto::ledger::{LedgerChannelGrant, LedgerHello, LedgerOutcome, LedgerRequest};
use tokio::sync::oneshot;

use crate::ledger_protocol::{LedgerClientMessage, LedgerCommand, LedgerReply, LedgerServerMessage};
use crate::{RetainOutcome, WriteOp};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(not(test))]
const OPERATION_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(test)]
// Covers archive compression and SQLite commit on loaded builders.
const OPERATION_TIMEOUT: Duration = Duration::from_secs(2);
// A full archive barrier includes the complete index backlog and fsync. Keep
// admissions short without abandoning a healthy durable commit mid-flight.
const DURABILITY_TIMEOUT: Duration = Duration::from_secs(capsem_proto::LEDGER_DURABILITY_TIMEOUT_SECONDS);
const BACKPRESSURE_MAX_WAIT: Duration = Duration::from_millis(5);

type ClientSender = ipc_channel::Sender<LedgerClientMessage>;
type ClientReceiver = ipc_channel::Receiver<LedgerServerMessage>;

enum AdmitReply {
    Async(oneshot::Sender<Result<(), String>>),
    Blocking(mpsc::SyncSender<Result<(), String>>),
}

impl AdmitReply {
    fn send(self, result: Result<(), String>) {
        match self {
            Self::Async(reply) => {
                let _ = reply.send(result);
            }
            Self::Blocking(reply) => {
                let _ = reply.send(result);
            }
        }
    }
}

enum RemoteMessage {
    Admit {
        event: Box<WriteOp>,
        reply: Option<AdmitReply>,
    },
    Flush {
        reply: oneshot::Sender<Result<(), String>>,
    },
    Retain {
        cutoff: String,
        reply: oneshot::Sender<Result<RetainOutcome, String>>,
    },
}

enum PendingReply {
    Admit(Option<AdmitReply>),
    Flush(oneshot::Sender<Result<(), String>>),
    Retain(oneshot::Sender<Result<RetainOutcome, String>>),
}

pub(super) struct RemoteWriter {
    tx: Mutex<Option<mpsc::SyncSender<RemoteMessage>>>,
    join: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl RemoteWriter {
    pub(super) fn start(stream: UnixStream, grant: LedgerChannelGrant, capacity: usize) -> io::Result<Self> {
        let abort = stream.try_clone()?;
        let (tx, rx) = mpsc::sync_channel(capacity.max(1));
        let (startup_tx, startup_rx) = mpsc::sync_channel(1);
        let join = std::thread::Builder::new()
            .name("capsem-ledger-client".into())
            .spawn(move || run(stream, grant, rx, startup_tx))?;
        match startup_rx.recv_timeout(STARTUP_TIMEOUT) {
            Ok(Ok(())) => Ok(Self {
                tx: Mutex::new(Some(tx)),
                join: Mutex::new(Some(join)),
            }),
            Ok(Err(error)) => {
                let _ = join.join();
                Err(io::Error::other(error))
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let _ = abort.shutdown(std::net::Shutdown::Both);
                let _ = join.join();
                Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "ledger client handshake timed out",
                ))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = join.join();
                Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "ledger client stopped during handshake",
                ))
            }
        }
    }

    pub(super) async fn admit(&self, event: WriteOp) -> Result<(), String> {
        let (reply, result) = oneshot::channel();
        self.send_async(RemoteMessage::Admit {
            event: Box::new(event),
            reply: Some(AdmitReply::Async(reply)),
        })
        .await?;
        result
            .await
            .map_err(|_| "ledger client stopped with admission pending".to_string())?
    }

    pub(super) fn try_admit(&self, event: WriteOp) -> bool {
        let Some(tx) = self.sender() else {
            return false;
        };
        tx.try_send(RemoteMessage::Admit {
            event: Box::new(event),
            reply: None,
        })
        .is_ok()
    }

    pub(super) fn admit_blocking(&self, event: WriteOp) -> Result<(), String> {
        let Some(tx) = self.sender() else {
            return Err("ledger client is shut down".to_string());
        };
        let (reply, result) = mpsc::sync_channel(1);
        tx.send(RemoteMessage::Admit {
            event: Box::new(event),
            reply: Some(AdmitReply::Blocking(reply)),
        })
        .map_err(|_| "ledger client channel closed".to_string())?;
        result
            .recv()
            .map_err(|_| "ledger client stopped with blocking admission pending".to_string())?
    }

    pub(super) async fn flush(&self) -> Result<(), String> {
        let (reply, result) = oneshot::channel();
        self.send_async(RemoteMessage::Flush { reply }).await?;
        result
            .await
            .map_err(|_| "ledger client stopped with flush pending".to_string())?
    }

    pub(super) async fn retain(&self, cutoff: &str) -> Result<RetainOutcome, String> {
        let (reply, result) = oneshot::channel();
        self.send_async(RemoteMessage::Retain {
            cutoff: cutoff.to_string(),
            reply,
        })
        .await?;
        result
            .await
            .map_err(|_| "ledger client stopped with retention pending".to_string())?
    }

    pub(super) fn shutdown(&self) {
        let _ = self.tx.lock().unwrap().take();
        if let Some(join) = self.join.lock().unwrap().take() {
            let _ = join.join();
        }
    }

    fn sender(&self) -> Option<mpsc::SyncSender<RemoteMessage>> {
        self.tx.lock().unwrap().clone()
    }

    async fn send_async(&self, mut message: RemoteMessage) -> Result<(), String> {
        let Some(tx) = self.sender() else {
            return Err("ledger client is shut down".to_string());
        };
        let mut wait = Duration::from_micros(50);
        let mut attempts = 0_u32;
        loop {
            match tx.try_send(message) {
                Ok(()) => return Ok(()),
                Err(mpsc::TrySendError::Full(returned)) => {
                    message = returned;
                    if attempts == 0 {
                        tokio::task::yield_now().await;
                    } else {
                        tokio::time::sleep(wait).await;
                        wait = (wait * 2).min(BACKPRESSURE_MAX_WAIT);
                    }
                    attempts += 1;
                }
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    return Err("ledger client channel closed".to_string());
                }
            }
        }
    }
}

fn run(
    stream: UnixStream,
    grant: LedgerChannelGrant,
    messages: mpsc::Receiver<RemoteMessage>,
    startup: mpsc::SyncSender<Result<(), String>>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = startup.send(Err(format!("build ledger client runtime: {error}")));
            return;
        }
    };
    let channel = {
        let _entered = runtime.enter();
        ipc_channel::channel_from_std::<LedgerClientMessage, LedgerServerMessage>(stream)
    };
    let (sender, receiver) = match channel {
        Ok(channel) => channel,
        Err(error) => {
            let _ = startup.send(Err(format!("open ledger client channel: {error}")));
            return;
        }
    };
    let ready = runtime.block_on(async {
        tokio::time::timeout(OPERATION_TIMEOUT, handshake(&sender, &receiver, grant))
            .await
            .map_err(|_| "ledger handshake timed out".to_string())?
    });
    if let Err(error) = ready {
        let _ = startup.send(Err(error));
        return;
    }
    if startup.send(Ok(())).is_err() {
        return;
    }
    let mut deferred = None;
    let mut request_id = 1_u64;
    loop {
        let first = match deferred.take().map_or_else(|| messages.recv(), Ok) {
            Ok(message) => message,
            Err(_) => {
                let result = runtime.block_on(async {
                    tokio::time::timeout(
                        DURABILITY_TIMEOUT,
                        dispatch(&sender, &receiver, request_id, LedgerCommand::Flush),
                    )
                    .await
                    .map_err(|_| "ledger shutdown flush timed out".to_string())?
                });
                if !matches!(result, Ok(LedgerReply::Durable { .. })) {
                    tracing::error!(?result, "ledger shutdown flush failed");
                }
                return;
            }
        };
        let mut batch = vec![first];
        if matches!(batch.first(), Some(RemoteMessage::Admit { .. })) {
            while batch.len() < capsem_proto::LEDGER_ADMISSION_BATCH_CAPACITY {
                let next = if batch.len() == 1 {
                    messages.recv_timeout(Duration::from_micros(
                        capsem_proto::LEDGER_ADMISSION_COALESCE_WAIT_MICROSECONDS,
                    ))
                } else {
                    messages.try_recv().map_err(|error| match error {
                        mpsc::TryRecvError::Empty => mpsc::RecvTimeoutError::Timeout,
                        mpsc::TryRecvError::Disconnected => mpsc::RecvTimeoutError::Disconnected,
                    })
                };
                match next {
                    Ok(message @ RemoteMessage::Admit { .. }) => batch.push(message),
                    Ok(message) => {
                        deferred = Some(message);
                        break;
                    }
                    Err(_) => break,
                }
            }
        }
        // Move events into their final command. Only small reply handles remain;
        // no event/body cloning, content hashes, reservations or anchor round trips.
        let (command, replies) = prepare(batch);
        if let Err(reason) = send_commands(&runtime, &sender, &receiver, &mut request_id, command, replies) {
            tracing::error!(error = %reason, "ledger client operation failed");
            if let Some(message) = deferred.take() {
                fail_message(message, &reason);
            }
            while let Ok(message) = messages.try_recv() {
                fail_message(message, &reason);
            }
            return;
        }
    }
}

fn send_commands(
    runtime: &tokio::runtime::Runtime,
    sender: &ClientSender,
    receiver: &ClientReceiver,
    request_id: &mut u64,
    command: LedgerCommand,
    replies: Vec<PendingReply>,
) -> Result<(), String> {
    let mut pending = vec![(command, replies)];
    while let Some((command, mut replies)) = pending.pop() {
        let timeout = match command {
            LedgerCommand::Flush | LedgerCommand::Retain { .. } => DURABILITY_TIMEOUT,
            _ => OPERATION_TIMEOUT,
        };
        let message = LedgerClientMessage::Request {
            request: LedgerRequest::new(*request_id, command).map_err(|error| error.to_string())?,
        };
        let frame = match ClientSender::prepare(&message) {
            Ok(frame) => frame,
            Err(error) => {
                let LedgerClientMessage::Request { request } = message else {
                    unreachable!()
                };
                if let LedgerCommand::AdmitBatch { mut events } = request.into_operation() {
                    if events.len() > 1 {
                        // Only rejected frames are encoded again. Split by event
                        // ownership, with no sizing pass or payload cloning.
                        let middle = events.len() / 2;
                        let right = events.split_off(middle);
                        let right_replies = replies.split_off(middle);
                        pending.push((LedgerCommand::AdmitBatch { events: right }, right_replies));
                        pending.push((LedgerCommand::AdmitBatch { events }, replies));
                        continue;
                    }
                }
                // Encoding wrote no socket bytes: refuse this operation while
                // retaining the channel and its next unused request id.
                tracing::error!(%error, "ledger operation rejected before socket write");
                for reply in replies {
                    finish(reply, Err(format!("encode ledger request: {error}")));
                }
                continue;
            }
        };
        drop(message);
        let result = runtime
            .block_on(async {
                tokio::time::timeout(timeout, dispatch_prepared(sender, receiver, *request_id, frame))
                    .await
                    .map_err(|_| "ledger client operation timed out".to_string())?
            })
            .and_then(|reply| {
                let expected = match (replies.first(), &reply) {
                    (Some(PendingReply::Admit(_)), LedgerReply::Accepted { .. }) => replies.len() == 1,
                    (Some(PendingReply::Admit(_)), LedgerReply::AcceptedBatch { accepted, .. }) => {
                        usize::from(*accepted) == replies.len()
                    }
                    (Some(PendingReply::Flush(_)), LedgerReply::Durable { .. }) => true,
                    (Some(PendingReply::Retain(_)), LedgerReply::Retained { .. }) => true,
                    _ => false,
                };
                if expected {
                    Ok(reply)
                } else {
                    Err("ledger returned an unexpected reply or admission count".into())
                }
            });
        let failure = result.as_ref().err().cloned();
        for reply in replies {
            finish(reply, result.clone());
        }
        let exhausted = request_id.checked_add(1);
        if let Some(reason) = failure.or_else(|| exhausted.is_none().then(|| "ledger request id exhausted".to_string()))
        {
            for (_, replies) in pending {
                for reply in replies {
                    finish(reply, Err(reason.clone()));
                }
            }
            return Err(reason);
        }
        *request_id = exhausted.expect("request id exhaustion handled above");
    }
    Ok(())
}

fn prepare(messages: Vec<RemoteMessage>) -> (LedgerCommand, Vec<PendingReply>) {
    let mut events = Vec::with_capacity(messages.len());
    let mut replies = Vec::with_capacity(messages.len());
    for message in messages {
        match message {
            RemoteMessage::Admit { event, reply } => {
                events.push(*event);
                replies.push(PendingReply::Admit(reply));
            }
            RemoteMessage::Flush { reply } => return (LedgerCommand::Flush, vec![PendingReply::Flush(reply)]),
            RemoteMessage::Retain { cutoff, reply } => {
                return (LedgerCommand::Retain { cutoff }, vec![PendingReply::Retain(reply)])
            }
        }
    }
    (LedgerCommand::AdmitBatch { events }, replies)
}

async fn handshake(sender: &ClientSender, receiver: &ClientReceiver, grant: LedgerChannelGrant) -> Result<(), String> {
    sender
        .send(LedgerClientMessage::Hello {
            hello: LedgerHello::for_grant(&grant),
        })
        .await
        .map_err(|error| format!("send ledger hello: {error}"))?;
    let message = receiver
        .recv()
        .await
        .map_err(|error| format!("receive ledger welcome: {error}"))?;
    let LedgerServerMessage::Welcome { welcome } = message else {
        return Err("ledger worker sent a response before welcome".to_string());
    };
    grant.validate_welcome(&welcome).map_err(|error| error.to_string())
}

async fn dispatch(
    sender: &ClientSender,
    receiver: &ClientReceiver,
    request_id: u64,
    command: LedgerCommand,
) -> Result<LedgerReply, String> {
    let message = LedgerClientMessage::Request {
        request: LedgerRequest::new(request_id, command).map_err(|error| error.to_string())?,
    };
    let frame = ClientSender::prepare(&message).map_err(|error| format!("encode ledger request: {error}"))?;
    drop(message);
    dispatch_prepared(sender, receiver, request_id, frame).await
}

async fn dispatch_prepared(
    sender: &ClientSender,
    receiver: &ClientReceiver,
    request_id: u64,
    frame: ipc_channel::PreparedFrame<LedgerClientMessage>,
) -> Result<LedgerReply, String> {
    sender
        .send_prepared(frame)
        .await
        .map_err(|error| format!("send ledger request: {error}"))?;
    let message = receiver
        .recv()
        .await
        .map_err(|error| format!("receive ledger response: {error}"))?;
    let LedgerServerMessage::Response { response } = message else {
        return Err("ledger worker repeated welcome".into());
    };
    if response.request_id() != request_id {
        return Err("ledger response id mismatch".into());
    }
    match response.outcome() {
        LedgerOutcome::Success { reply } => {
            reply.validate().map_err(|error| error.to_string())?;
            Ok(reply.clone())
        }
        LedgerOutcome::Failure { failure } => Err(format!("ledger {:?}: {}", failure.code(), failure.message())),
    }
}

fn finish(reply: PendingReply, result: Result<LedgerReply, String>) {
    match reply {
        PendingReply::Admit(reply) => {
            let result = result.and_then(|reply| match reply {
                LedgerReply::Accepted { .. } | LedgerReply::AcceptedBatch { .. } => Ok(()),
                reply => Err(format!("ledger returned {reply:?} for admission")),
            });
            if let Some(reply) = reply {
                reply.send(result);
            }
        }
        PendingReply::Flush(reply) => {
            let result = result.and_then(|reply| match reply {
                LedgerReply::Durable { .. } => Ok(()),
                reply => Err(format!("ledger returned {reply:?} for flush")),
            });
            let _ = reply.send(result);
        }
        PendingReply::Retain(reply) => {
            let result = result.and_then(|reply| match reply {
                LedgerReply::Retained { outcome } => Ok(outcome),
                reply => Err(format!("ledger returned {reply:?} for retention")),
            });
            let _ = reply.send(result);
        }
    }
}

fn fail_message(message: RemoteMessage, reason: &str) {
    let (_, replies) = prepare(vec![message]);
    for reply in replies {
        finish(reply, Err(reason.to_string()));
    }
}

#[cfg(test)]
mod tests;
