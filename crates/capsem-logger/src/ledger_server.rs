//! Storage-owning dispatch for one session ledger process.

use std::io;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use capsem_archive::ArchiveCodecs;
use capsem_foundation::ipc_channel;
use capsem_proto::ledger::{
    LedgerChannelGrant, LedgerFailure, LedgerFailureCode, LedgerRequest, LedgerResponse, LedgerWelcome,
};

use crate::db::PreparedDbHandle;
use crate::ledger_protocol::{
    LedgerBodyMetadata, LedgerClientMessage, LedgerCommand, LedgerExportSummary, LedgerReply, LedgerServerMessage,
    MAX_LEDGER_STREAM_CHUNK_BYTES,
};
use crate::{DbHandle, StoredBody};

mod queries;

type ServerSender = ipc_channel::Sender<LedgerServerMessage>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LedgerClientExit {
    Disconnected,
    ShutdownRequested,
}

#[derive(Default)]
struct AdmissionState {
    last_accepted: u64,
}

/// One process-owned session database shared by its authenticated clients.
pub struct LedgerServer {
    db: Arc<DbHandle>,
    session_dir: PathBuf,
    admission: tokio::sync::Mutex<AdmissionState>,
}

/// A ledger whose files, schema and exclusive writer authority are prepared,
/// but whose database workers have not started yet. Confining worker processes
/// between [`LedgerServer::prepare_with_codecs`] and [`Self::start`] ensures
/// every database thread inherits the process sandbox.
pub struct PreparedLedgerServer {
    db: PreparedDbHandle,
    session_dir: PathBuf,
}

impl LedgerServer {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        Self::open_with_codecs(path, ArchiveCodecs::default())
    }

    pub fn open_with_codecs(path: &Path, codecs: ArchiveCodecs) -> rusqlite::Result<Self> {
        Self::prepare_with_codecs(path, codecs).map(PreparedLedgerServer::start)
    }

    pub fn prepare_with_codecs(path: &Path, codecs: ArchiveCodecs) -> rusqlite::Result<PreparedLedgerServer> {
        Ok(PreparedLedgerServer {
            db: DbHandle::prepare_with_codecs(path, codecs)?,
            session_dir: path.parent().unwrap_or_else(|| Path::new(".")).to_path_buf(),
        })
    }

    /// Serve one descriptor whose expected authority came from the trusted
    /// coordinator. Other clients may call this concurrently on the same
    /// server; SQLite and archive ownership remain in this process.
    pub async fn serve_client(
        &self,
        stream: UnixStream,
        grant: LedgerChannelGrant,
    ) -> Result<LedgerClientExit, String> {
        let (sender, receiver) =
            ipc_channel::channel_from_std::<LedgerServerMessage, LedgerClientMessage>(stream).map_err(io_string)?;
        let first = receiver.recv().await.map_err(io_string)?;
        let LedgerClientMessage::Hello { hello } = first else {
            return Err("ledger client sent an operation before hello".into());
        };
        grant.validate_hello(&hello).map_err(|error| error.to_string())?;
        sender
            .send(LedgerServerMessage::Welcome {
                welcome: LedgerWelcome::for_grant(&grant),
            })
            .await
            .map_err(io_string)?;

        loop {
            let message = match receiver.recv().await {
                Ok(message) => message,
                Err(error) if disconnected(&error) => return Ok(LedgerClientExit::Disconnected),
                Err(error) => return Err(error.to_string()),
            };
            let LedgerClientMessage::Request { request } = message else {
                return Err("ledger client repeated hello".into());
            };
            let request_id = request.request_id();
            if let Err(error) = request.authorize(&grant) {
                let code = if matches!(error, capsem_proto::ledger::LedgerProtocolError::Unauthorized { .. }) {
                    LedgerFailureCode::UnauthorizedOperation
                } else {
                    LedgerFailureCode::InvalidRequest
                };
                send_failure(&sender, request_id, code, error.to_string()).await?;
                continue;
            }
            if self.dispatch(&sender, request, grant).await? {
                return Ok(LedgerClientExit::ShutdownRequested);
            }
        }
    }

    async fn dispatch(
        &self,
        sender: &ServerSender,
        request: LedgerRequest<LedgerCommand>,
        grant: LedgerChannelGrant,
    ) -> Result<bool, String> {
        let request_id = request.request_id();
        let result = match request.into_operation() {
            LedgerCommand::Admit { event, commitment } => {
                let admission_id = {
                    let mut admission = self.admission.lock().await;
                    match commitment.validate_grant(grant) {
                        Ok(()) => match self.db.write_committed(*event, commitment).await {
                            Ok(()) => admission
                                .last_accepted
                                .checked_add(1)
                                .ok_or_else(|| "ledger admission id exhausted".to_string())
                                .inspect(|next| admission.last_accepted = *next),
                            Err(error) => Err(storage(error)),
                        },
                        Err(error) => Err(error.to_string()),
                    }
                };
                match admission_id {
                    Ok(admission_id) => send_success(sender, request_id, LedgerReply::Accepted { admission_id }).await,
                    Err(error) => Err(error),
                }
            }
            LedgerCommand::Flush => {
                // The lock orders the writer enqueue and this barrier against
                // every client, so the returned id cannot name a later write.
                let through_admission_id = {
                    let admission = self.admission.lock().await;
                    match self.db.flush().await {
                        Ok(()) => Ok(admission.last_accepted),
                        Err(error) => Err(storage(error)),
                    }
                };
                match through_admission_id {
                    Ok(through_admission_id) => {
                        send_success(sender, request_id, LedgerReply::Durable { through_admission_id }).await
                    }
                    Err(error) => Err(error),
                }
            }
            LedgerCommand::Query { query } => match queries::execute(&self.db, query).await {
                Ok(sets) => send_success(sender, request_id, LedgerReply::Query { sets }).await,
                Err(error) => Err(storage(error)),
            },
            LedgerCommand::ReadBodies { event_id } => match self.db.read_bodies(&event_id).await {
                Ok(bodies) => self.send_bodies(sender, request_id, bodies).await,
                Err(error) => Err(storage(error)),
            },
            LedgerCommand::Counters => match self.current_counters().await {
                Ok(counters) => {
                    send_success(
                        sender,
                        request_id,
                        LedgerReply::Counters {
                            counters: Box::new((*counters).clone()),
                        },
                    )
                    .await
                }
                Err(error) => Err(storage(error)),
            },
            LedgerCommand::Retain { cutoff } => match self.db.retain_bodies_since(&cutoff).await {
                Ok(outcome) => send_success(sender, request_id, LedgerReply::Retained { outcome }).await,
                Err(error) => Err(storage(error)),
            },
            LedgerCommand::ExportWarc => self.send_warc(sender, request_id).await,
            LedgerCommand::Snapshot { snapshot_id } => {
                let destination = self.session_dir.join(snapshot_directory_name(snapshot_id));
                let source = self.session_dir.clone();
                let admission = self.admission.lock().await;
                let snapshot = match self.db.flush().await {
                    Ok(()) => tokio::task::spawn_blocking(move || {
                        crate::snapshot_session_ledger(&source, &destination).map_err(storage)
                    })
                    .await
                    .map_err(|error| format!("ledger snapshot task failed: {error}"))?,
                    Err(error) => Err(storage(error)),
                };
                drop(admission);
                match snapshot {
                    Ok(()) => send_success(sender, request_id, LedgerReply::Snapshotted).await,
                    Err(error) => Err(error),
                }
            }
            LedgerCommand::Shutdown => {
                send_success(sender, request_id, LedgerReply::ShuttingDown).await?;
                return Ok(true);
            }
        };
        if let Err(error) = result {
            send_failure(sender, request_id, LedgerFailureCode::Storage, &error).await?;
        }
        Ok(false)
    }

    async fn current_counters(&self) -> Result<Arc<capsem_proto::ledger_counters::LedgerCounters>, String> {
        self.db.ready().await?;
        self.db.ledger_counters().await
    }

    async fn send_bodies(&self, sender: &ServerSender, request_id: u64, bodies: Vec<StoredBody>) -> Result<(), String> {
        for (index, body) in bodies.into_iter().enumerate() {
            let body_id = u32::try_from(index + 1).map_err(|_| "too many bodies in one ledger reply")?;
            let metadata = LedgerBodyMetadata {
                event_id: body.event_id,
                source_table: body.source_table,
                direction: body.direction,
                content_type: body.content_type,
                original_bytes: body.original_bytes,
                stored_bytes: body.bytes.len() as u64,
                truncated: body.truncated,
                body_hash: body.body_hash,
            };
            send_success(sender, request_id, LedgerReply::BodyStart { body_id, metadata }).await?;
            let mut offset = 0_u64;
            for bytes in body.bytes.chunks(MAX_LEDGER_STREAM_CHUNK_BYTES) {
                send_success(
                    sender,
                    request_id,
                    LedgerReply::BodyChunk {
                        body_id,
                        offset,
                        bytes: bytes.to_vec(),
                    },
                )
                .await?;
                offset += bytes.len() as u64;
            }
        }
        send_success(sender, request_id, LedgerReply::BodiesComplete).await
    }

    async fn send_warc(&self, sender: &ServerSender, request_id: u64) -> Result<(), String> {
        let (chunks, mut receiver) = tokio::sync::mpsc::channel(4);
        let db = Arc::clone(&self.db);
        let export = tokio::spawn(async move { db.export_warc(WarcChunkWriter { chunks }).await });
        let mut offset = 0_u64;
        while let Some(bytes) = receiver.recv().await {
            let chunk_len = bytes.len() as u64;
            if let Err(error) = send_success(sender, request_id, LedgerReply::WarcChunk { offset, bytes }).await {
                drop(receiver);
                let _ = export.await;
                return Err(error);
            }
            offset += chunk_len;
        }
        let summary = export
            .await
            .map_err(|error| format!("WARC export task failed: {error}"))?
            .map_err(storage)?;
        let skipped_by_reason = summary
            .counts_by_reason()
            .into_iter()
            .map(|(reason, count)| (reason.to_string(), count))
            .collect();
        send_success(
            sender,
            request_id,
            LedgerReply::WarcComplete {
                summary: LedgerExportSummary {
                    records: summary.records,
                    bytes_written: summary.bytes_written,
                    skipped_count: summary.skipped_count,
                    skipped_by_reason,
                },
            },
        )
        .await
    }
}

impl PreparedLedgerServer {
    pub fn start(self) -> LedgerServer {
        LedgerServer {
            db: Arc::new(self.db.start()),
            session_dir: self.session_dir,
            admission: tokio::sync::Mutex::new(AdmissionState::default()),
        }
    }
}

#[must_use]
pub fn snapshot_directory_name(snapshot_id: [u8; 16]) -> String {
    format!(".ledger-snapshot-{}", uuid::Uuid::from_bytes(snapshot_id).simple())
}

/// Execute the same bounded named intent as a descriptor client against an
/// already-owned handle. Embedded tests and the worker dispatch share this
/// rail so neither can grow a second SQL definition.
pub async fn execute_named_query(
    db: &DbHandle,
    query: crate::ledger_protocol::LedgerQuery,
) -> Result<Vec<crate::ledger_protocol::LedgerRows>, String> {
    queries::execute(db, query).await
}

struct WarcChunkWriter {
    chunks: tokio::sync::mpsc::Sender<Vec<u8>>,
}

impl io::Write for WarcChunkWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        for chunk in buffer.chunks(MAX_LEDGER_STREAM_CHUNK_BYTES) {
            self.chunks
                .blocking_send(chunk.to_vec())
                .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "ledger WARC client disconnected"))?;
        }
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

async fn send_success(sender: &ServerSender, request_id: u64, reply: LedgerReply) -> Result<(), String> {
    reply.validate().map_err(|error| error.to_string())?;
    let response = LedgerResponse::success(request_id, reply).map_err(|error| error.to_string())?;
    sender
        .send(LedgerServerMessage::Response { response })
        .await
        .map_err(io_string)
}

async fn send_failure(
    sender: &ServerSender,
    request_id: u64,
    code: LedgerFailureCode,
    message: impl AsRef<str>,
) -> Result<(), String> {
    let message = message.as_ref();
    let bounded = &message[..floor_char_boundary(message, capsem_proto::ledger::MAX_LEDGER_ERROR_BYTES)];
    let failure = LedgerFailure::new(code, bounded).map_err(|error| error.to_string())?;
    let response = LedgerResponse::failure(request_id, failure).map_err(|error| error.to_string())?;
    sender
        .send(LedgerServerMessage::Response { response })
        .await
        .map_err(io_string)
}

fn floor_char_boundary(value: &str, at: usize) -> usize {
    let mut at = at.min(value.len());
    while !value.is_char_boundary(at) {
        at -= 1;
    }
    at
}

fn disconnected(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::UnexpectedEof | io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset
    )
}

fn io_string(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn storage(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests;
