//! Descriptor-backed writer client for the dedicated ledger owner.

use std::io;
use std::os::unix::net::UnixStream;
use std::sync::{mpsc, Mutex};
use std::time::Duration;

use capsem_foundation::ipc_channel;
use capsem_proto::ledger::{LedgerChannelGrant, LedgerHello, LedgerOutcome, LedgerRequest};
use capsem_proto::ledger_commitment::{
    CommitmentClientMessage, CommitmentCommand, CommitmentReply, CommitmentReservation, CommitmentServerMessage,
    LedgerCommitment, MAX_COMMITMENTS_PER_CHECKPOINT, ZERO_COMMITMENT_HASH,
};
use tokio::sync::oneshot;

use crate::ledger_protocol::{LedgerAdmission, LedgerClientMessage, LedgerCommand, LedgerReply, LedgerServerMessage};
use crate::{RetainOutcome, WriteOp};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(not(test))]
const OPERATION_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(test)]
// Large body admission includes framing, archive compression, SQLite commit,
// and its producer commitment. Two hundred milliseconds made the WARC
// backpressure regression fail during fixture setup on otherwise healthy
// macOS and loaded Linux builders.
const OPERATION_TIMEOUT: Duration = Duration::from_secs(2);
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

enum AdmissionMessage {
    Operations {
        messages: Vec<RemoteMessage>,
        commitments: Vec<LedgerCommitment>,
    },
    Shutdown,
}

enum AdmissionResult {
    Operations {
        messages: Vec<RemoteMessage>,
        commitments: Vec<LedgerCommitment>,
        result: Result<LedgerReply, String>,
    },
    Shutdown(Result<LedgerReply, String>),
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
        tokio::time::timeout(
            OPERATION_TIMEOUT,
            commitment_handshake(&commitment_sender, &commitment_receiver, grant),
        )
        .await
        .map_err(|_| "ledger commitment handshake timed out".to_string())?
    });
    if let Err(error) = handshake {
        let _ = startup.send(Err(error));
        return;
    }
    let (admission_tx, admission_rx) = mpsc::sync_channel(1);
    let (result_tx, result_rx) = mpsc::channel();
    let (admission_startup_tx, admission_startup_rx) = mpsc::sync_channel(1);
    let admission_abort = match stream.try_clone() {
        Ok(abort) => abort,
        Err(error) => {
            let _ = startup.send(Err(format!("clone ledger client channel: {error}")));
            return;
        }
    };
    let admission = match std::thread::Builder::new()
        .name("capsem-ledger-admission".into())
        .spawn(move || admission_run(stream, grant, admission_rx, result_tx, admission_startup_tx))
    {
        Ok(admission) => admission,
        Err(error) => {
            let _ = startup.send(Err(format!("spawn ledger admission stage: {error}")));
            return;
        }
    };
    match admission_startup_rx.recv_timeout(STARTUP_TIMEOUT) {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            let _ = admission.join();
            let _ = startup.send(Err(error));
            return;
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            let _ = admission_abort.shutdown(std::net::Shutdown::Both);
            let _ = admission.join();
            let _ = startup.send(Err("ledger admission stage startup timed out".into()));
            return;
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            let _ = admission.join();
            let _ = startup.send(Err("ledger admission stage stopped during startup".into()));
            return;
        }
    }
    if startup.send(Ok(())).is_err() {
        return;
    }

    let mut commitments = CommitmentState::new(commitment_sender, commitment_receiver, grant);
    let mut failure = None;
    let mut deferred = None;
    loop {
        while let Ok(result) = result_rx.try_recv() {
            if let Err(error) = apply_admission_result(&runtime, &mut commitments, result) {
                failure = Some(error);
                break;
            }
        }
        if failure.is_some() {
            break;
        }
        let message = match deferred
            .take()
            .map_or_else(|| messages.recv_timeout(Duration::from_millis(1)), Ok)
        {
            Ok(message) => message,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                if admission_tx.send(AdmissionMessage::Shutdown).is_err() {
                    failure = Some("ledger admission stage stopped before shutdown flush".into());
                    break;
                }
                loop {
                    match result_rx.recv_timeout(OPERATION_TIMEOUT) {
                        Ok(AdmissionResult::Shutdown(result)) => {
                            if let Err(error) = apply_shutdown_result(&runtime, &mut commitments, result) {
                                tracing::error!(%error, "ledger client shutdown left an unanchored tail");
                            }
                            break;
                        }
                        Ok(result) => {
                            if let Err(error) = apply_admission_result(&runtime, &mut commitments, result) {
                                tracing::error!(%error, "ledger client stopped while draining admissions");
                                break;
                            }
                        }
                        Err(error) => {
                            tracing::error!(%error, "ledger admission stage did not complete shutdown");
                            break;
                        }
                    }
                }
                drop(admission_tx);
                let _ = admission.join();
                return;
            }
        };
        let mut batch = vec![message];
        if matches!(batch.first(), Some(RemoteMessage::Admit { .. })) {
            while batch.len() < capsem_proto::LEDGER_ADMISSION_BATCH_CAPACITY {
                match messages.try_recv() {
                    Ok(message @ RemoteMessage::Admit { .. }) => batch.push(message),
                    Ok(message) => {
                        deferred = Some(message);
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) if batch.len() == 1 => {
                        match messages.recv_timeout(Duration::from_micros(
                            capsem_proto::LEDGER_ADMISSION_COALESCE_WAIT_MICROSECONDS,
                        )) {
                            Ok(message @ RemoteMessage::Admit { .. }) => batch.push(message),
                            Ok(message) => {
                                deferred = Some(message);
                                break;
                            }
                            Err(_) => break,
                        }
                    }
                    Err(_) => break,
                }
            }
        }
        let reserved = if matches!(batch.first(), Some(RemoteMessage::Admit { .. })) {
            let events = batch
                .iter()
                .map(|message| match message {
                    RemoteMessage::Admit { event, .. } => event.as_ref(),
                    _ => unreachable!("admission batch contains only events"),
                })
                .collect::<Vec<_>>();
            match runtime.block_on(async {
                tokio::time::timeout(OPERATION_TIMEOUT, commitments.reserve_batch(&events))
                    .await
                    .map_err(|_| "ledger commitment reservation timed out".to_string())?
            }) {
                Ok(commitments) => commitments,
                Err(error) => {
                    finish_all(batch, Err(error.clone()));
                    failure = Some(error);
                    break;
                }
            }
        } else {
            Vec::new()
        };
        if let Err(error) = admission_tx.send(AdmissionMessage::Operations {
            messages: batch,
            commitments: reserved,
        }) {
            let AdmissionMessage::Operations { messages, .. } = error.0 else {
                unreachable!("only operation messages are sent here")
            };
            let reason = "ledger admission stage stopped".to_string();
            finish_all(messages, Err(reason.clone()));
            failure = Some(reason);
            break;
        }
    }
    drop(admission_tx);
    let _ = admission_abort.shutdown(std::net::Shutdown::Both);
    let _ = admission.join();
    let reason = failure.unwrap_or_else(|| "ledger client stopped".into());
    tracing::error!(error = %reason, "ledger client operation failed");
    while let Ok(result) = result_rx.try_recv() {
        fail_admission_result(result, &reason);
    }
    fail_pending(&messages, &reason);
}

fn admission_run(
    stream: UnixStream,
    grant: LedgerChannelGrant,
    messages: mpsc::Receiver<AdmissionMessage>,
    results: mpsc::Sender<AdmissionResult>,
    startup: mpsc::SyncSender<Result<(), String>>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = startup.send(Err(format!("build ledger admission runtime: {error}")));
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
        let (result, response) = match message {
            AdmissionMessage::Operations { messages, commitments } => {
                let result = runtime.block_on(async {
                    tokio::time::timeout(
                        OPERATION_TIMEOUT,
                        dispatch_admissions(&sender, &receiver, request_id, &messages, &commitments),
                    )
                    .await
                    .map_err(|_| "ledger client operation timed out".to_string())?
                });
                let failed = result.is_err();
                (
                    failed,
                    AdmissionResult::Operations {
                        messages,
                        commitments,
                        result,
                    },
                )
            }
            AdmissionMessage::Shutdown => {
                let result = runtime.block_on(async {
                    tokio::time::timeout(OPERATION_TIMEOUT, flush_admissions(&sender, &receiver, request_id))
                        .await
                        .map_err(|_| "ledger client shutdown flush timed out".to_string())?
                });
                let _ = results.send(AdmissionResult::Shutdown(result));
                return;
            }
        };
        if results.send(response).is_err() {
            return;
        }
        if result {
            while let Ok(message) = messages.try_recv() {
                fail_admission_message(
                    message,
                    &results,
                    "ledger admission stage stopped after an operation failed",
                );
            }
            return;
        }
        let Some(next) = request_id.checked_add(1) else {
            while let Ok(message) = messages.try_recv() {
                fail_admission_message(message, &results, "ledger request id exhausted");
            }
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

    async fn reserve_batch(&mut self, events: &[&WriteOp]) -> Result<Vec<LedgerCommitment>, String> {
        if events.is_empty() || events.len() > capsem_proto::LEDGER_ADMISSION_BATCH_CAPACITY {
            return Err("ledger commitment reservation batch is empty or exceeds its bound".into());
        }
        let first_producer_sequence = self
            .producer_sequence
            .checked_add(1)
            .ok_or_else(|| "ledger producer sequence exhausted".to_string())?;
        let reservations = events
            .iter()
            .map(|event| {
                CommitmentReservation::new(event.kind(), super::operation::commitment_event_hash(event)?)
                    .map_err(|error| error.to_string())
            })
            .collect::<Result<Vec<_>, String>>()?;
        let reply = self
            .request(CommitmentCommand::ReserveBatch {
                first_producer_sequence,
                previous_hash: self.previous_hash,
                reservations: reservations.clone(),
            })
            .await?;
        let CommitmentReply::ReservedBatch { global_sequences } = reply else {
            return Err(format!(
                "ledger commitment coordinator returned {reply:?} for reservation batch"
            ));
        };
        if global_sequences.len() != reservations.len() {
            return Err("ledger commitment coordinator returned the wrong batch length".into());
        }
        let mut producer_sequence = first_producer_sequence;
        let mut previous_hash = self.previous_hash;
        let mut commitments = Vec::with_capacity(events.len());
        for (reservation, global_sequence) in reservations.into_iter().zip(global_sequences) {
            let commitment = LedgerCommitment::new(
                self.grant,
                producer_sequence,
                global_sequence,
                reservation.event_kind(),
                reservation.event_hash(),
                previous_hash,
            )
            .map_err(|error| error.to_string())?;
            previous_hash = commitment.commitment_hash();
            commitments.push(commitment);
            producer_sequence = producer_sequence
                .checked_add(1)
                .ok_or_else(|| "ledger producer sequence exhausted".to_string())?;
        }
        self.producer_sequence = producer_sequence - 1;
        self.previous_hash = previous_hash;
        Ok(commitments)
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

async fn dispatch_admissions(
    sender: &ClientSender,
    receiver: &ClientReceiver,
    request_id: u64,
    messages: &[RemoteMessage],
    commitments: &[LedgerCommitment],
) -> Result<LedgerReply, String> {
    let command = match messages {
        [RemoteMessage::Flush { .. }] if commitments.is_empty() => LedgerCommand::Flush,
        [RemoteMessage::Retain { cutoff, .. }] if commitments.is_empty() => {
            LedgerCommand::Retain { cutoff: cutoff.clone() }
        }
        _ if messages.len() == commitments.len() && !messages.is_empty() => {
            let admissions = messages
                .iter()
                .zip(commitments)
                .map(|(message, commitment)| {
                    let RemoteMessage::Admit { event, .. } = message else {
                        return Err("ledger admission batch contains a barrier".to_string());
                    };
                    Ok(LedgerAdmission::new(event.as_ref().clone(), commitment.clone()))
                })
                .collect::<Result<Vec<_>, String>>()?;
            LedgerCommand::AdmitBatch { admissions }
        }
        _ => return Err("ledger operation batch has inconsistent commitments".into()),
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

async fn flush_admissions(
    sender: &ClientSender,
    receiver: &ClientReceiver,
    request_id: u64,
) -> Result<LedgerReply, String> {
    dispatch_admissions(
        sender,
        receiver,
        request_id,
        &[RemoteMessage::Flush {
            reply: oneshot::channel().0,
        }],
        &[],
    )
    .await
}

fn apply_admission_result(
    runtime: &tokio::runtime::Runtime,
    commitments: &mut CommitmentState,
    result: AdmissionResult,
) -> Result<(), String> {
    let AdmissionResult::Operations {
        messages,
        commitments: reserved,
        result,
    } = result
    else {
        return Err("ledger admission stage sent an unexpected shutdown result".into());
    };
    let result = match (result, reserved.as_slice()) {
        (
            Ok(LedgerReply::AcceptedBatch {
                first_admission_id,
                through_admission_id,
                accepted,
            }),
            reserved,
        ) if usize::from(accepted) == reserved.len() => {
            commitments
                .pending
                .extend((first_admission_id..=through_admission_id).zip(reserved.iter().cloned()));
            Ok(LedgerReply::AcceptedBatch {
                first_admission_id,
                through_admission_id,
                accepted,
            })
        }
        (Ok(LedgerReply::Durable { through_admission_id }), []) => {
            runtime.block_on(async {
                tokio::time::timeout(OPERATION_TIMEOUT, commitments.anchor_through(through_admission_id))
                    .await
                    .map_err(|_| "ledger commitment checkpoint timed out".to_string())?
            })?;
            Ok(LedgerReply::Durable { through_admission_id })
        }
        (Ok(reply @ LedgerReply::Retained { .. }), []) => Ok(reply),
        (Ok(reply), _) => Err(format!("ledger returned {reply:?} for the staged operation")),
        (Err(error), _) => Err(error),
    };
    let failed = result.as_ref().err().cloned();
    finish_all(messages, result);
    failed.map_or(Ok(()), Err)
}

fn apply_shutdown_result(
    runtime: &tokio::runtime::Runtime,
    commitments: &mut CommitmentState,
    result: Result<LedgerReply, String>,
) -> Result<(), String> {
    let LedgerReply::Durable { through_admission_id } = result? else {
        return Err("ledger returned a non-durable shutdown response".into());
    };
    runtime.block_on(async {
        tokio::time::timeout(OPERATION_TIMEOUT, commitments.anchor_through(through_admission_id))
            .await
            .map_err(|_| "ledger commitment shutdown checkpoint timed out".to_string())?
    })
}

fn fail_admission_message(message: AdmissionMessage, results: &mpsc::Sender<AdmissionResult>, reason: &str) {
    match message {
        AdmissionMessage::Operations { messages, commitments } => {
            let _ = results.send(AdmissionResult::Operations {
                messages,
                commitments,
                result: Err(reason.into()),
            });
        }
        AdmissionMessage::Shutdown => {
            let _ = results.send(AdmissionResult::Shutdown(Err(reason.into())));
        }
    }
}

fn fail_admission_result(result: AdmissionResult, reason: &str) {
    if let AdmissionResult::Operations { messages, .. } = result {
        finish_all(messages, Err(reason.into()));
    }
}

fn finish_all(messages: Vec<RemoteMessage>, result: Result<LedgerReply, String>) {
    for message in messages {
        finish(message, result.clone());
    }
}

fn finish(message: RemoteMessage, result: Result<LedgerReply, String>) {
    match message {
        RemoteMessage::Admit { reply, .. } => {
            let result = result.and_then(|reply| match reply {
                LedgerReply::Accepted { .. } | LedgerReply::AcceptedBatch { .. } => Ok(()),
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
                CommitmentCommand::ReserveBatch {
                    first_producer_sequence,
                    previous_hash,
                    reservations,
                } => {
                    let mut previous_hash = previous_hash;
                    let mut sequences = Vec::with_capacity(reservations.len());
                    let mut failed = None;
                    for (producer_sequence, reservation) in (first_producer_sequence..).zip(reservations) {
                        global_sequence += 1;
                        match LedgerCommitment::new(
                            grant,
                            producer_sequence,
                            global_sequence,
                            reservation.event_kind(),
                            reservation.event_hash(),
                            previous_hash,
                        ) {
                            Ok(commitment) => {
                                previous_hash = commitment.commitment_hash();
                                pending.push(commitment);
                                sequences.push(global_sequence);
                            }
                            Err(error) => {
                                failed = Some(error.to_string());
                                break;
                            }
                        }
                    }
                    failed.map_or(
                        CommitmentReply::ReservedBatch {
                            global_sequences: sequences,
                        },
                        |message| CommitmentReply::Failed { message },
                    )
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
