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
const OPERATION_TIMEOUT: Duration = Duration::from_millis(200);
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
    let handshake = runtime.block_on(async {
        tokio::time::timeout(OPERATION_TIMEOUT, handshake(&sender, &receiver, grant))
            .await
            .map_err(|_| "ledger client handshake timed out".to_string())?
    });
    if let Err(error) = handshake {
        let _ = startup.send(Err(error));
        return;
    }
    if startup.send(Ok(())).is_err() {
        return;
    }

    let mut request_id = 1_u64;
    while let Ok(message) = messages.recv() {
        let result = runtime.block_on(async {
            tokio::time::timeout(OPERATION_TIMEOUT, dispatch(&sender, &receiver, request_id, &message))
                .await
                .map_err(|_| "ledger client operation timed out".to_string())?
        });
        finish(message, result.clone());
        if let Err(error) = result {
            tracing::error!(%error, "ledger client operation failed");
            fail_pending(&messages, &error);
            return;
        }
        let Some(next) = request_id.checked_add(1) else {
            fail_pending(&messages, "ledger request id exhausted");
            return;
        };
        request_id = next;
    }
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
    message: &RemoteMessage,
) -> Result<LedgerReply, String> {
    let command = match message {
        RemoteMessage::Admit { event, .. } => LedgerCommand::Admit { event: event.clone() },
        RemoteMessage::Flush { .. } => LedgerCommand::Flush,
        RemoteMessage::Retain { cutoff, .. } => LedgerCommand::Retain { cutoff: cutoff.clone() },
    };
    sender
        .send(LedgerClientMessage::Request {
            request: LedgerRequest::new(request_id, command).map_err(|error| error.to_string())?,
        })
        .await
        .map_err(|error| format!("send ledger request: {error}"))?;
    let message = receiver
        .recv()
        .await
        .map_err(|error| format!("receive ledger response: {error}"))?;
    let LedgerServerMessage::Response { response } = message else {
        return Err("ledger worker repeated welcome".to_string());
    };
    if response.request_id() != request_id {
        return Err(format!(
            "ledger response id {} does not match {request_id}",
            response.request_id()
        ));
    }
    match response.outcome() {
        LedgerOutcome::Success { reply } => {
            reply.validate().map_err(|error| error.to_string())?;
            Ok(reply.clone())
        }
        LedgerOutcome::Failure { failure } => Err(format!("ledger {:?}: {}", failure.code(), failure.message())),
    }
}

fn finish(message: RemoteMessage, result: Result<LedgerReply, String>) {
    match message {
        RemoteMessage::Admit { reply, .. } => {
            let result = result.and_then(|reply| match reply {
                LedgerReply::Accepted { .. } => Ok(()),
                reply => Err(format!("ledger returned {reply:?} for admission")),
            });
            if let Some(reply) = reply {
                reply.send(result);
            }
        }
        RemoteMessage::Flush { reply } => {
            let result = result.and_then(|response| match response {
                LedgerReply::Durable { .. } => Ok(()),
                response => Err(format!("ledger returned {response:?} for flush")),
            });
            let _ = reply.send(result);
        }
        RemoteMessage::Retain { reply, .. } => {
            let result = result.and_then(|response| match response {
                LedgerReply::Retained { outcome } => Ok(outcome),
                response => Err(format!("ledger returned {response:?} for retention")),
            });
            let _ = reply.send(result);
        }
    }
}

fn fail_pending(messages: &mpsc::Receiver<RemoteMessage>, reason: &str) {
    while let Ok(message) = messages.try_recv() {
        finish(message, Err(reason.to_string()));
    }
}

#[cfg(test)]
mod tests;
