//! Cancellation-safe client for one authenticated session-ledger channel.

use std::collections::BTreeMap;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use capsem_foundation::ipc_channel;
use capsem_proto::ledger::{LedgerChannelGrant, LedgerClientRole, LedgerHello, LedgerOutcome, LedgerRequest};
use capsem_proto::ledger_counters::LedgerCounters;
use tokio::sync::{mpsc, oneshot};

use crate::ledger_protocol::{
    LedgerBodyMetadata, LedgerClientMessage, LedgerCommand, LedgerExportSummary, LedgerQuery, LedgerReply, LedgerRows,
    LedgerServerMessage,
};
use crate::{RetainOutcome, StoredBody};

const COMMAND_CAPACITY: usize = 16;
const EXPORT_CHUNK_CAPACITY: usize = 4;
#[cfg(not(test))]
const OPERATION_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(test)]
const OPERATION_TIMEOUT: Duration = Duration::from_secs(1);
#[cfg(not(test))]
const STREAM_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(test)]
const STREAM_TIMEOUT: Duration = Duration::from_millis(250);
#[cfg(not(test))]
const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(5 * 60 + 5);
#[cfg(test)]
const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(5);

type ClientSender = ipc_channel::Sender<LedgerClientMessage>;
type ClientReceiver = ipc_channel::Receiver<LedgerServerMessage>;

enum Command {
    Query {
        query: LedgerQuery,
        reply: oneshot::Sender<Result<Vec<LedgerRows>, String>>,
    },
    Counters {
        reply: oneshot::Sender<Result<Arc<LedgerCounters>, String>>,
    },
    ReadBodies {
        event_id: String,
        reply: oneshot::Sender<Result<Vec<StoredBody>, String>>,
    },
    Retain {
        cutoff: String,
        reply: oneshot::Sender<Result<RetainOutcome, String>>,
    },
    ExportWarc {
        reply: oneshot::Sender<Result<LedgerWarcExport, String>>,
    },
    Snapshot {
        snapshot_id: [u8; 16],
        reply: oneshot::Sender<Result<(), String>>,
    },
}

/// One authenticated, serial connection to a dedicated session-ledger owner.
///
/// The background actor owns all framing. Dropping a caller future only drops
/// its reply channel; the actor still consumes the complete worker response.
#[derive(Clone)]
pub struct LedgerClient {
    commands: mpsc::Sender<Command>,
    path: Arc<PathBuf>,
    role: LedgerClientRole,
    read_cache_epoch: Arc<AtomicU64>,
}

impl LedgerClient {
    pub async fn connect(stream: UnixStream, grant: LedgerChannelGrant, logical_path: PathBuf) -> Result<Self, String> {
        let (sender, receiver) = ipc_channel::channel_from_std::<LedgerClientMessage, LedgerServerMessage>(stream)
            .map_err(|error| format!("open ledger client channel: {error}"))?;
        tokio::time::timeout(OPERATION_TIMEOUT, handshake(&sender, &receiver, grant))
            .await
            .map_err(|_| "ledger client handshake timed out".to_string())??;

        let (commands, inbox) = mpsc::channel(COMMAND_CAPACITY);
        let read_cache_epoch = Arc::new(AtomicU64::new(0));
        tokio::spawn(run_actor(sender, receiver, inbox, Arc::clone(&read_cache_epoch)));
        Ok(Self {
            commands,
            path: Arc::new(logical_path),
            role: grant.role(),
            read_cache_epoch,
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        self.path.as_path()
    }

    #[must_use]
    pub const fn role(&self) -> LedgerClientRole {
        self.role
    }

    #[must_use]
    pub fn read_cache_epoch(&self) -> u64 {
        self.read_cache_epoch.load(Ordering::Acquire)
    }

    pub async fn query(&self, query: LedgerQuery) -> Result<Vec<LedgerRows>, String> {
        let (reply, result) = oneshot::channel();
        self.send(Command::Query { query, reply }).await?;
        receive_result(result, "query").await
    }

    pub async fn counters(&self) -> Result<Arc<LedgerCounters>, String> {
        let (reply, result) = oneshot::channel();
        self.send(Command::Counters { reply }).await?;
        receive_result(result, "counters").await
    }

    pub async fn read_bodies(&self, event_id: &str) -> Result<Vec<StoredBody>, String> {
        let (reply, result) = oneshot::channel();
        self.send(Command::ReadBodies {
            event_id: event_id.to_string(),
            reply,
        })
        .await?;
        receive_result(result, "body read").await
    }

    pub async fn retain_bodies_since(&self, cutoff: &str) -> Result<RetainOutcome, String> {
        let (reply, result) = oneshot::channel();
        self.send(Command::Retain {
            cutoff: cutoff.to_string(),
            reply,
        })
        .await?;
        receive_result(result, "retention").await
    }

    pub async fn export_warc(&self) -> Result<LedgerWarcExport, String> {
        let (reply, result) = oneshot::channel();
        self.send(Command::ExportWarc { reply }).await?;
        receive_result(result, "WARC export").await
    }

    pub async fn snapshot(&self, snapshot_id: [u8; 16]) -> Result<PathBuf, String> {
        let (reply, result) = oneshot::channel();
        self.send(Command::Snapshot { snapshot_id, reply }).await?;
        receive_result(result, "snapshot").await?;
        Ok(self
            .path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(crate::ledger_server::snapshot_directory_name(snapshot_id)))
    }

    async fn send(&self, command: Command) -> Result<(), String> {
        self.commands
            .send(command)
            .await
            .map_err(|_| "ledger client channel closed".to_string())
    }
}

/// A bounded WARC byte stream followed by its worker-produced summary.
pub struct LedgerWarcExport {
    chunks: mpsc::Receiver<Result<Vec<u8>, String>>,
    completion: oneshot::Receiver<Result<LedgerExportSummary, String>>,
}

impl LedgerWarcExport {
    pub async fn next_chunk(&mut self) -> Option<Result<Vec<u8>, String>> {
        self.chunks.recv().await
    }

    pub async fn finish(mut self) -> Result<LedgerExportSummary, String> {
        while self.chunks.recv().await.is_some() {}
        self.completion
            .await
            .map_err(|_| "ledger client stopped with WARC export pending".to_string())?
    }
}

async fn receive_result<T>(result: oneshot::Receiver<Result<T, String>>, operation: &str) -> Result<T, String> {
    result
        .await
        .map_err(|_| format!("ledger client stopped with {operation} pending"))?
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

async fn run_actor(
    sender: ClientSender,
    receiver: ClientReceiver,
    mut commands: mpsc::Receiver<Command>,
    read_cache_epoch: Arc<AtomicU64>,
) {
    let mut request_id = 1_u64;
    while let Some(command) = commands.recv().await {
        let result = dispatch(&sender, &receiver, request_id, command, &read_cache_epoch).await;
        if let Err(error) = result {
            tracing::error!(%error, "ledger client operation failed");
            fail_pending(&mut commands, &error);
            return;
        }
        let Some(next) = request_id.checked_add(1) else {
            fail_pending(&mut commands, "ledger request id exhausted");
            return;
        };
        request_id = next;
    }
}

async fn dispatch(
    sender: &ClientSender,
    receiver: &ClientReceiver,
    request_id: u64,
    command: Command,
    read_cache_epoch: &AtomicU64,
) -> Result<(), String> {
    match command {
        Command::Query { query, reply } => {
            let result = one_reply(sender, receiver, request_id, LedgerCommand::Query { query })
                .await
                .and_then(|response| match response {
                    LedgerReply::Query { sets } => Ok(sets),
                    response => Err(unexpected("query", response)),
                });
            finish(reply, result)
        }
        Command::Counters { reply } => {
            let result = one_reply(sender, receiver, request_id, LedgerCommand::Counters)
                .await
                .and_then(|response| match response {
                    LedgerReply::Counters {
                        counters,
                        read_cache_epoch: remote_epoch,
                    } => {
                        read_cache_epoch.store(remote_epoch, Ordering::Release);
                        Ok(Arc::new(*counters))
                    }
                    response => Err(unexpected("counters", response)),
                });
            finish(reply, result)
        }
        Command::ReadBodies { event_id, reply } => {
            let result = read_bodies(sender, receiver, request_id, &event_id).await;
            finish(reply, result)
        }
        Command::Retain { cutoff, reply } => {
            let result = one_reply(sender, receiver, request_id, LedgerCommand::Retain { cutoff })
                .await
                .and_then(|response| match response {
                    LedgerReply::Retained { outcome } => Ok(outcome),
                    response => Err(unexpected("retention", response)),
                });
            finish(reply, result)
        }
        Command::ExportWarc { reply } => export_warc(sender, receiver, request_id, reply).await,
        Command::Snapshot { snapshot_id, reply } => {
            let result = one_reply_with_timeout(
                sender,
                receiver,
                request_id,
                LedgerCommand::Snapshot { snapshot_id },
                SNAPSHOT_TIMEOUT,
            )
            .await
            .and_then(|response| match response {
                LedgerReply::Snapshotted => Ok(()),
                response => Err(unexpected("snapshot", response)),
            });
            finish(reply, result)
        }
    }
}

fn finish<T>(reply: oneshot::Sender<Result<T, String>>, result: Result<T, String>) -> Result<(), String> {
    let failure = result.as_ref().err().cloned();
    let _ = reply.send(result);
    match failure {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

async fn send_request(sender: &ClientSender, request_id: u64, command: LedgerCommand) -> Result<(), String> {
    sender
        .send(LedgerClientMessage::Request {
            request: LedgerRequest::new(request_id, command).map_err(|error| error.to_string())?,
        })
        .await
        .map_err(|error| format!("send ledger request: {error}"))
}

async fn one_reply(
    sender: &ClientSender,
    receiver: &ClientReceiver,
    request_id: u64,
    command: LedgerCommand,
) -> Result<LedgerReply, String> {
    one_reply_with_timeout(sender, receiver, request_id, command, OPERATION_TIMEOUT).await
}

async fn one_reply_with_timeout(
    sender: &ClientSender,
    receiver: &ClientReceiver,
    request_id: u64,
    command: LedgerCommand,
    timeout: Duration,
) -> Result<LedgerReply, String> {
    send_request(sender, request_id, command).await?;
    tokio::time::timeout(timeout, receive_reply(receiver, request_id))
        .await
        .map_err(|_| "ledger client operation timed out".to_string())?
}

async fn receive_reply(receiver: &ClientReceiver, request_id: u64) -> Result<LedgerReply, String> {
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

struct BodyAssembly {
    metadata: LedgerBodyMetadata,
    bytes: Vec<u8>,
}

async fn read_bodies(
    sender: &ClientSender,
    receiver: &ClientReceiver,
    request_id: u64,
    event_id: &str,
) -> Result<Vec<StoredBody>, String> {
    send_request(
        sender,
        request_id,
        LedgerCommand::ReadBodies {
            event_id: event_id.to_string(),
        },
    )
    .await?;
    let mut bodies = BTreeMap::<u32, BodyAssembly>::new();
    loop {
        let reply = tokio::time::timeout(OPERATION_TIMEOUT, receive_reply(receiver, request_id))
            .await
            .map_err(|_| "ledger body stream timed out".to_string())??;
        match reply {
            LedgerReply::BodyStart { body_id, metadata } => {
                if metadata.event_id != event_id {
                    return Err("ledger body event id does not match the request".to_string());
                }
                let capacity = usize::try_from(metadata.stored_bytes)
                    .map_err(|_| "ledger body length does not fit memory".to_string())?;
                if bodies
                    .insert(
                        body_id,
                        BodyAssembly {
                            metadata,
                            bytes: Vec::with_capacity(capacity),
                        },
                    )
                    .is_some()
                {
                    return Err("ledger repeated a body id".to_string());
                }
            }
            LedgerReply::BodyChunk { body_id, offset, bytes } => {
                let body = bodies
                    .get_mut(&body_id)
                    .ok_or_else(|| "ledger sent a body chunk before its metadata".to_string())?;
                if offset != body.bytes.len() as u64 {
                    return Err("ledger body chunk offset is not contiguous".to_string());
                }
                let new_len = body
                    .bytes
                    .len()
                    .checked_add(bytes.len())
                    .ok_or_else(|| "ledger body length overflowed".to_string())?;
                if new_len as u64 > body.metadata.stored_bytes {
                    return Err("ledger body exceeded its declared length".to_string());
                }
                body.bytes.extend_from_slice(&bytes);
            }
            LedgerReply::BodiesComplete => return finish_bodies(bodies),
            response => return Err(unexpected("body read", response)),
        }
    }
}

fn finish_bodies(bodies: BTreeMap<u32, BodyAssembly>) -> Result<Vec<StoredBody>, String> {
    bodies
        .into_values()
        .map(|body| {
            if body.bytes.len() as u64 != body.metadata.stored_bytes {
                return Err("ledger body ended before its declared length".to_string());
            }
            let hash = format!("blake3:{}", blake3::hash(&body.bytes).to_hex());
            if hash != body.metadata.body_hash {
                return Err("ledger body hash does not match its bytes".to_string());
            }
            Ok(StoredBody {
                event_id: body.metadata.event_id,
                source_table: body.metadata.source_table,
                direction: body.metadata.direction,
                content_type: body.metadata.content_type,
                original_bytes: body.metadata.original_bytes,
                truncated: body.metadata.truncated,
                body_hash: body.metadata.body_hash,
                bytes: body.bytes,
            })
        })
        .collect()
}

async fn export_warc(
    sender: &ClientSender,
    receiver: &ClientReceiver,
    request_id: u64,
    reply: oneshot::Sender<Result<LedgerWarcExport, String>>,
) -> Result<(), String> {
    send_request(sender, request_id, LedgerCommand::ExportWarc).await?;
    let (chunks, chunk_rx) = mpsc::channel(EXPORT_CHUNK_CAPACITY);
    let (completion, completion_rx) = oneshot::channel();
    let export = LedgerWarcExport {
        chunks: chunk_rx,
        completion: completion_rx,
    };
    let mut consumer_open = reply.send(Ok(export)).is_ok();
    let mut offset = 0_u64;
    loop {
        let response = tokio::time::timeout(STREAM_TIMEOUT, receive_reply(receiver, request_id))
            .await
            .map_err(|_| "ledger WARC stream timed out".to_string())?;
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                if consumer_open {
                    let _ = forward_export_chunk(&chunks, Err(error.clone())).await;
                }
                let _ = completion.send(Err(error.clone()));
                return Err(error);
            }
        };
        match response {
            LedgerReply::WarcChunk { offset: actual, bytes } => {
                if actual != offset {
                    let error = "ledger WARC chunk offset is not contiguous".to_string();
                    if consumer_open {
                        let _ = forward_export_chunk(&chunks, Err(error.clone())).await;
                    }
                    let _ = completion.send(Err(error.clone()));
                    return Err(error);
                }
                offset += bytes.len() as u64;
                if consumer_open && !forward_export_chunk(&chunks, Ok(bytes)).await {
                    consumer_open = false;
                }
            }
            LedgerReply::WarcComplete { summary } => {
                if summary.bytes_written != offset {
                    let error = "ledger WARC summary byte count does not match the stream".to_string();
                    let _ = completion.send(Err(error.clone()));
                    return Err(error);
                }
                drop(chunks);
                let _ = completion.send(Ok(summary));
                return Ok(());
            }
            response => {
                let error = unexpected("WARC export", response);
                let _ = completion.send(Err(error.clone()));
                return Err(error);
            }
        }
    }
}

async fn forward_export_chunk(chunks: &mpsc::Sender<Result<Vec<u8>, String>>, chunk: Result<Vec<u8>, String>) -> bool {
    matches!(
        tokio::time::timeout(STREAM_TIMEOUT, chunks.send(chunk)).await,
        Ok(Ok(()))
    )
}

fn unexpected(operation: &str, response: LedgerReply) -> String {
    format!("ledger returned {response:?} for {operation}")
}

fn fail_pending(commands: &mut mpsc::Receiver<Command>, reason: &str) {
    while let Ok(command) = commands.try_recv() {
        match command {
            Command::Query { reply, .. } => {
                let _ = reply.send(Err(reason.to_string()));
            }
            Command::Counters { reply } => {
                let _ = reply.send(Err(reason.to_string()));
            }
            Command::ReadBodies { reply, .. } => {
                let _ = reply.send(Err(reason.to_string()));
            }
            Command::Retain { reply, .. } => {
                let _ = reply.send(Err(reason.to_string()));
            }
            Command::ExportWarc { reply } => {
                let _ = reply.send(Err(reason.to_string()));
            }
            Command::Snapshot { reply, .. } => {
                let _ = reply.send(Err(reason.to_string()));
            }
        }
    }
}

#[cfg(test)]
mod tests;
