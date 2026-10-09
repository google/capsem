//! Descriptor-backed writer client for the dedicated ledger owner.

use std::io;
use std::os::unix::net::UnixStream;
use std::sync::{mpsc, Mutex};
use std::time::Duration;

use capsem_foundation::ipc_channel;
use capsem_proto::ledger::{LedgerChannelGrant, LedgerHello, LedgerOutcome, LedgerRequest};
use capsem_proto::ledger_commitment::{
    CommitmentClientMessage, CommitmentCommand, CommitmentReply, CommitmentServerMessage, LedgerCommitment,
    ZERO_COMMITMENT_HASH,
};
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
type CommitmentSender = ipc_channel::Sender<CommitmentClientMessage>;
type CommitmentReceiver = ipc_channel::Receiver<CommitmentServerMessage>;

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
    pub(super) fn start(
        stream: UnixStream,
        commitment_stream: UnixStream,
        grant: LedgerChannelGrant,
        capacity: usize,
    ) -> io::Result<Self> {
        let abort = stream.try_clone()?;
        let (tx, rx) = mpsc::sync_channel(capacity.max(1));
        let (startup_tx, startup_rx) = mpsc::sync_channel(1);
        let join = std::thread::Builder::new()
            .name("capsem-ledger-client".into())
            .spawn(move || run(stream, commitment_stream, grant, rx, startup_tx))?;
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
    commitment_stream: UnixStream,
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
    let commitment_channel = {
        let _entered = runtime.enter();
        ipc_channel::channel_from_std::<CommitmentClientMessage, CommitmentServerMessage>(commitment_stream)
    };
    let (commitment_sender, commitment_receiver) = match commitment_channel {
        Ok(channel) => channel,
        Err(error) => {
            let _ = startup.send(Err(format!("open ledger commitment channel: {error}")));
            return;
        }
    };
    let handshake = runtime.block_on(async {
        tokio::time::timeout(OPERATION_TIMEOUT, async {
            handshake(&sender, &receiver, grant).await?;
            commitment_handshake(&commitment_sender, &commitment_receiver, grant).await
        })
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
    let mut commitments = CommitmentState::new(commitment_sender, commitment_receiver, grant);
    while let Ok(message) = messages.recv() {
        let result = runtime.block_on(async {
            tokio::time::timeout(
                OPERATION_TIMEOUT,
                dispatch(&sender, &receiver, request_id, &message, &mut commitments),
            )
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
    if !commitments.pending.is_empty() {
        let result = runtime.block_on(async {
            tokio::time::timeout(
                OPERATION_TIMEOUT,
                flush_and_anchor(&sender, &receiver, request_id, &mut commitments),
            )
            .await
            .map_err(|_| "ledger client shutdown flush timed out".to_string())?
        });
        if let Err(error) = result {
            tracing::error!(%error, "ledger client shutdown left an unanchored tail");
        }
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

async fn commitment_handshake(
    sender: &CommitmentSender,
    receiver: &CommitmentReceiver,
    grant: LedgerChannelGrant,
) -> Result<(), String> {
    sender
        .send(CommitmentClientMessage::Hello {
            hello: LedgerHello::for_grant(&grant),
        })
        .await
        .map_err(|error| format!("send ledger commitment hello: {error}"))?;
    let CommitmentServerMessage::Welcome { welcome } = receiver
        .recv()
        .await
        .map_err(|error| format!("receive ledger commitment welcome: {error}"))?
    else {
        return Err("ledger commitment coordinator sent a response before welcome".into());
    };
    grant.validate_welcome(&welcome).map_err(|error| error.to_string())
}

struct CommitmentState {
    sender: CommitmentSender,
    receiver: CommitmentReceiver,
    grant: LedgerChannelGrant,
    request_id: u64,
    producer_sequence: u64,
    previous_hash: [u8; 32],
    pending: Vec<(u64, LedgerCommitment)>,
}

impl CommitmentState {
    fn new(sender: CommitmentSender, receiver: CommitmentReceiver, grant: LedgerChannelGrant) -> Self {
        Self {
            sender,
            receiver,
            grant,
            request_id: 1,
            producer_sequence: 0,
            previous_hash: ZERO_COMMITMENT_HASH,
            pending: Vec::new(),
        }
    }

    async fn request(&mut self, command: CommitmentCommand) -> Result<CommitmentReply, String> {
        let request_id = self.request_id;
        self.sender
            .send(CommitmentClientMessage::Request { request_id, command })
            .await
            .map_err(|error| format!("send ledger commitment request: {error}"))?;
        let CommitmentServerMessage::Response {
            request_id: response_id,
            reply,
        } = self
            .receiver
            .recv()
            .await
            .map_err(|error| format!("receive ledger commitment response: {error}"))?
        else {
            return Err("ledger commitment coordinator repeated welcome".into());
        };
        if response_id != request_id {
            return Err("ledger commitment response id mismatch".into());
        }
        self.request_id = request_id
            .checked_add(1)
            .ok_or_else(|| "ledger commitment request id exhausted".to_string())?;
        match reply {
            CommitmentReply::Failed { message } => Err(format!("ledger commitment coordinator: {message}")),
            reply => Ok(reply),
        }
    }

    async fn reserve(&mut self, event: &WriteOp) -> Result<LedgerCommitment, String> {
        let producer_sequence = self
            .producer_sequence
            .checked_add(1)
            .ok_or_else(|| "ledger producer sequence exhausted".to_string())?;
        let event_hash = super::operation::commitment_event_hash(event)?;
        let reply = self
            .request(CommitmentCommand::Reserve {
                producer_sequence,
                event_kind: event.kind().to_string(),
                event_hash,
                previous_hash: self.previous_hash,
            })
            .await?;
        let CommitmentReply::Reserved { global_sequence } = reply else {
            return Err(format!(
                "ledger commitment coordinator returned {reply:?} for reservation"
            ));
        };
        let commitment = LedgerCommitment::new(
            self.grant,
            producer_sequence,
            global_sequence,
            event.kind(),
            event_hash,
            self.previous_hash,
        )
        .map_err(|error| error.to_string())?;
        self.producer_sequence = producer_sequence;
        self.previous_hash = commitment.commitment_hash();
        Ok(commitment)
    }

    async fn cancel(&mut self, commitment: &LedgerCommitment) -> Result<(), String> {
        let reply = self
            .request(CommitmentCommand::Cancel {
                global_sequence: commitment.global_sequence(),
            })
            .await?;
        if !matches!(reply, CommitmentReply::Canceled) {
            return Err(format!(
                "ledger commitment coordinator returned {reply:?} for cancellation"
            ));
        }
        self.producer_sequence = self.producer_sequence.saturating_sub(1);
        self.previous_hash = commitment.previous_hash();
        Ok(())
    }

    async fn anchor_through(&mut self, through_admission_id: u64) -> Result<(), String> {
        let count = self
            .pending
            .iter()
            .take_while(|(admission_id, _)| *admission_id <= through_admission_id)
            .count();
        if count == 0 {
            return Ok(());
        }
        let commitments = self.pending[..count]
            .iter()
            .map(|(_, commitment)| commitment.clone())
            .collect();
        let reply = self.request(CommitmentCommand::Anchor { commitments }).await?;
        if !matches!(reply, CommitmentReply::Anchored { .. }) {
            return Err(format!(
                "ledger commitment coordinator returned {reply:?} for checkpoint"
            ));
        }
        self.pending.drain(..count);
        Ok(())
    }
}

async fn dispatch(
    sender: &ClientSender,
    receiver: &ClientReceiver,
    request_id: u64,
    message: &RemoteMessage,
    commitments: &mut CommitmentState,
) -> Result<LedgerReply, String> {
    let reserved = match message {
        RemoteMessage::Admit { event, .. } => Some(commitments.reserve(event).await?),
        _ => None,
    };
    let command = match message {
        RemoteMessage::Admit { event, .. } => LedgerCommand::Admit {
            event: event.clone(),
            commitment: reserved.clone().expect("admission reserved above"),
        },
        RemoteMessage::Flush { .. } => LedgerCommand::Flush,
        RemoteMessage::Retain { cutoff, .. } => LedgerCommand::Retain { cutoff: cutoff.clone() },
    };
    let response = async {
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
    .await;
    match (response, reserved) {
        (Ok(LedgerReply::Accepted { admission_id }), Some(commitment)) => {
            commitments.pending.push((admission_id, commitment));
            Ok(LedgerReply::Accepted { admission_id })
        }
        (Ok(LedgerReply::Durable { through_admission_id }), None) => {
            commitments.anchor_through(through_admission_id).await?;
            Ok(LedgerReply::Durable { through_admission_id })
        }
        (Err(error), Some(commitment)) => {
            let cancellation = commitments.cancel(&commitment).await;
            Err(match cancellation {
                Ok(()) => error,
                Err(cancel) => format!("{error}; commitment cancellation failed: {cancel}"),
            })
        }
        (result, _) => result,
    }
}

async fn flush_and_anchor(
    sender: &ClientSender,
    receiver: &ClientReceiver,
    request_id: u64,
    commitments: &mut CommitmentState,
) -> Result<(), String> {
    let reply = dispatch(
        sender,
        receiver,
        request_id,
        &RemoteMessage::Flush {
            reply: oneshot::channel().0,
        },
        commitments,
    )
    .await?;
    if matches!(reply, LedgerReply::Durable { .. }) {
        Ok(())
    } else {
        Err(format!("ledger returned {reply:?} for shutdown flush"))
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
pub(crate) fn test_commitment_channel(
    grant: LedgerChannelGrant,
) -> (UnixStream, tokio::task::JoinHandle<Result<(), String>>) {
    let (client, server) = UnixStream::pair().unwrap();
    let task = tokio::spawn(async move {
        let (sender, receiver) =
            ipc_channel::channel_from_std::<CommitmentServerMessage, CommitmentClientMessage>(server)
                .map_err(|error| error.to_string())?;
        let CommitmentClientMessage::Hello { hello } = receiver.recv().await.map_err(|error| error.to_string())? else {
            return Err("test commitment client omitted hello".into());
        };
        grant.validate_hello(&hello).map_err(|error| error.to_string())?;
        sender
            .send(CommitmentServerMessage::Welcome {
                welcome: capsem_proto::ledger::LedgerWelcome::for_grant(&grant),
            })
            .await
            .map_err(|error| error.to_string())?;
        let mut global_sequence = 0_u64;
        let mut pending = Vec::<LedgerCommitment>::new();
        loop {
            let CommitmentClientMessage::Request { request_id, command } =
                receiver.recv().await.map_err(|error| error.to_string())?
            else {
                return Err("test commitment client repeated hello".into());
            };
            let reply = match command {
                CommitmentCommand::Reserve {
                    producer_sequence,
                    event_kind,
                    event_hash,
                    previous_hash,
                } => {
                    global_sequence += 1;
                    match LedgerCommitment::new(
                        grant,
                        producer_sequence,
                        global_sequence,
                        event_kind,
                        event_hash,
                        previous_hash,
                    ) {
                        Ok(commitment) => {
                            pending.push(commitment);
                            CommitmentReply::Reserved { global_sequence }
                        }
                        Err(error) => CommitmentReply::Failed {
                            message: error.to_string(),
                        },
                    }
                }
                CommitmentCommand::Cancel { global_sequence } => {
                    if pending.last().map(LedgerCommitment::global_sequence) == Some(global_sequence) {
                        pending.pop();
                        CommitmentReply::Canceled
                    } else {
                        CommitmentReply::Failed {
                            message: "cancellation did not name pending tail".into(),
                        }
                    }
                }
                CommitmentCommand::Anchor { commitments } => {
                    if commitments.len() <= pending.len() && pending[..commitments.len()] == commitments {
                        pending.drain(..commitments.len());
                        CommitmentReply::Anchored { checkpoint_sequence: 1 }
                    } else {
                        CommitmentReply::Failed {
                            message: "checkpoint did not match pending prefix".into(),
                        }
                    }
                }
            };
            sender
                .send(CommitmentServerMessage::Response { request_id, reply })
                .await
                .map_err(|error| error.to_string())?;
        }
    });
    (client, task)
}

#[cfg(test)]
mod tests;
