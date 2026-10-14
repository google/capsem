use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant, SystemTime};

use capsem_archive::ArchiveCodecs;
use capsem_telemetry::db::{DB_MEMORY_UNFLUSHED_OPS, DB_SHUTDOWN_FLUSH_MS};
use rusqlite::{params, Connection, ErrorCode, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use tracing::{error, warn};

use crate::counters::{self, LedgerCounters, LedgerTally};
use crate::events::{
    AuditEvent, DnsEvent, ExecEvent, ExecEventComplete, FileEvent, HostEvent, McpCall, ModelCall, NetEvent,
    NetworkDeparture, NetworkMembership, NetworkRecord, PolicyMutationEvent, SecurityAskEvent, SecurityDecisionEvent,
    SecurityRuleEvent, SubstitutionEvent, TransportEvent,
};
use crate::schema;

mod bodies;
mod flush;
mod flush_faults;
#[cfg(test)]
use flush::flush_dirty_tables_to_disk;
use flush::{flush_dirty_tables_to_disk_recorded, FlushRequest};
mod legacy;
mod model_rows;
mod producer;
#[cfg(test)]
pub(crate) use bodies::close_blocks_at_every_flush_for_tests;
pub(crate) use bodies::{archive_lock_path_for_db, archive_path_for_db};
use bodies::{BodyArchive, EventBodyBlob};
use flush_faults::take_disk_flush_failure_for_tests;
#[cfg(test)]
pub(crate) use flush_faults::{
    fail_disk_flushes_for_path_for_tests, fail_disk_flushes_for_tests, DISK_FLUSH_FAULT_LOCK,
};
use model_rows::insert_model_call;

/// Maximum bytes stored for any non-preview text field (256 KB). Callers
/// should truncate before constructing events, but the logger enforces this
/// defensively to prevent unbounded storage.
const MAX_FIELD_BYTES: usize = 256 * 1024;

/// Maximum bytes stored for a request or response header blob (16 KB).
///
/// Headers used to share `MAX_FIELD_BYTES` with model text, and 256 KB is not
/// a bound for them -- it is a budget an upstream can spend. Real headers
/// average around 300 bytes, so the gap between what they need and what they
/// were allowed was three orders of magnitude, and every byte of it went
/// straight into the hot in-RAM mirror that two processes held. A server that
/// wants the ledger to cost a gigabyte only has to pad a response header and
/// be talked to four thousand times.
///
/// 16 KB is above every header set worth recording and below the point where
/// padding pays. `net_events.headers_truncated` records that something was
/// cut, because a header blob that stops mid-line must not read as one that
/// simply ended there. It is one flag for the row, not one per direction: a
/// reader that sees it set knows the request headers, the response headers or
/// both were cut, and must compare each blob's length against the cap to say
/// which. Splitting it in two would be the honest shape if anything ever needs
/// to tell them apart; nothing does yet, and a column nobody reads is the
/// thing this work has been removing.
const HEADER_BYTES: usize = 16 * 1024;

/// Display previews are a UI convenience; the forensic copy is the archived
/// body. 2 KB shows the first screen of any JSON or SSE body. A 10-day
/// session once carried 75 MB of "previews" averaging 28 KB, mirrored into
/// RAM by two processes on top of the identical bytes stored beside them.
pub(crate) const PREVIEW_BYTES: usize = 2 * 1024;
/// The most one archived body stores; a producer that retains more has
/// nowhere to put it. Guest exec capture sizes its per-lane buffer by this.
pub const MAX_BODY_BLOB_BYTES: usize = 10 * 1024 * 1024;
const DEFAULT_BATCH_CAPACITY: usize = 10_000;
/// Unflushed ops that force a disk flush before the interval is up.
///
/// Every op waiting for a flush is a row held in RAM, and every flush empties
/// that memory, so what the writer holds is bounded by `DISK_FLUSH_INTERVAL`
/// of traffic, not by the session. This threshold only caps a burst faster
/// than the interval. It stays high on purpose: at 10,000 a 100k-row burst
/// spent its accept path waiting on ten disk flushes and was accepted 3-4x
/// slower (see `benches/db_read_write_micro.rs`), where the whole point of
/// the memory schema is that accepting a row never waits on the disk.
const DISK_FLUSH_THRESHOLD_OPS: usize = capsem_proto::LEDGER_DISK_FLUSH_MAX_PENDING_EVENTS;
const DISK_FLUSH_INTERVAL: Duration = Duration::from_millis(capsem_proto::LEDGER_DISK_FLUSH_INTERVAL_MILLISECONDS);

#[derive(Clone, Copy)]
struct WriterSchedule {
    capacity: usize,
    interval: Duration,
}

impl WriterSchedule {
    fn new(capacity: usize) -> Self {
        Self {
            capacity: if capacity == 0 {
                DEFAULT_BATCH_CAPACITY
            } else {
                capacity
            },
            interval: DISK_FLUSH_INTERVAL,
        }
    }
}

pub const DB_ENQUEUE_SPAN: &str = "capsem.db.enqueue";
pub const DB_WRITE_BATCH_SPAN: &str = "capsem.db.write_batch";
pub const DB_SHUTDOWN_FLUSH_SPAN: &str = "capsem.db.shutdown_flush";

/// What the writer reads `event_body_blobs.created_at` and
/// `body_blocks.sealed_at` from.
///
/// Every other value the archive index holds is derived from the body it
/// names, so this is the one thing a replay of the same session cannot
/// reproduce. Injecting it is what lets the fixture regenerator rebuild a
/// ledger that matches the one it read.
pub type LedgerClock = fn() -> SystemTime;

static IN_MEMORY_WRITER_ID: AtomicU64 = AtomicU64::new(0);

fn new_event_id() -> String {
    let value = capsem_crypto::uuid_v4().simple().to_string();
    value[..12].to_string()
}

pub(crate) fn format_timestamp(timestamp: SystemTime) -> String {
    format_ledger_timestamp(timestamp)
}

/// How every ledger timestamp is spelled: RFC 3339, UTC, fixed-width to the
/// microsecond.
///
/// Public because a caller that compares against one -- a retention cutoff
/// against `body_blocks.sealed_at` -- is comparing strings, and the ordering
/// only holds while both have the same shape. `12:00:00Z` sorts *after*
/// `12:00:00.000001Z`, so a cutoff formatted without the fraction would keep
/// exactly the blocks it meant to drop.
#[must_use]
pub fn format_ledger_timestamp(timestamp: SystemTime) -> String {
    humantime::format_rfc3339_micros(timestamp).to_string()
}

/// Truncate an optional string field to at most `max` bytes, at a char
/// boundary so the result stays valid UTF-8.
fn cap_bytes(s: &Option<String>, max: usize) -> Option<String> {
    s.as_ref().map(|v| {
        if v.len() <= max {
            v.clone()
        } else {
            let mut end = max;
            while end > 0 && !v.is_char_boundary(end) {
                end -= 1;
            }
            v[..end].to_string()
        }
    })
}

/// Truncate an optional string field to MAX_FIELD_BYTES.
fn cap_field(s: &Option<String>) -> Option<String> {
    cap_bytes(s, MAX_FIELD_BYTES)
}

/// Truncate a stored header blob to HEADER_BYTES, saying whether it was cut.
///
/// The caller ORs the two answers into the row's single `headers_truncated`.
///
/// The flag is the point: a reader looking at a header set that ends mid-line
/// cannot otherwise tell a hostile 256 KB pad from a short response, and a
/// forensic record that silently loses its tail is worse than one that admits
/// to it.
pub(crate) fn cap_headers(s: &Option<String>) -> (Option<String>, bool) {
    let truncated = s.as_ref().is_some_and(|v| v.len() > HEADER_BYTES);
    (cap_bytes(s, HEADER_BYTES), truncated)
}

/// Truncate an optional display-preview field to PREVIEW_BYTES. The full
/// body, when one exists, is in the archive and `event_body_blobs` says
/// where; this only bounds the compact copy shown in a UI list.
pub(crate) fn cap_preview(s: &Option<String>) -> Option<String> {
    cap_bytes(s, PREVIEW_BYTES)
}

/// Derive the display preview of a captured body.
///
/// The event carries the body once, as bytes; this is the only place that
/// turns it into the compact string a UI list shows. Only the leading
/// `PREVIEW_BYTES` are converted -- a 10 MiB body must not be transcoded in
/// full to produce 2 KiB of it -- and the second cap absorbs the replacement
/// character a cut multi-byte sequence expands into.
pub(crate) fn body_preview(body: Option<&[u8]>) -> Option<String> {
    body.filter(|bytes| !bytes.is_empty()).map(output_preview)
}

/// The display preview of captured output, empty output included.
///
/// One definition for every consumer of a preview: the ledger column and the
/// security-rule input both read this, so a rule never sees more than a UI
/// list shows and neither pays for transcoding the whole body.
pub fn output_preview(bytes: &[u8]) -> String {
    let head = &bytes[..bytes.len().min(PREVIEW_BYTES)];
    cap_bytes(&Some(String::from_utf8_lossy(head).into_owned()), PREVIEW_BYTES).unwrap_or_default()
}

fn blake3_ref(value: &str) -> String {
    format!("blake3:{}", capsem_crypto::blake3::hash(value.as_bytes()).to_hex())
}

fn blake3_bytes_ref(value: &[u8]) -> String {
    format!("blake3:{}", capsem_crypto::blake3::hash(value).to_hex())
}

/// Typed write operations sent to the writer thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WriteOp {
    TransportEvent(TransportEvent),
    NetEvent(NetEvent),
    ModelCall(ModelCall),
    McpCall(McpCall),
    FileEvent(FileEvent),
    ExecEvent(ExecEvent),
    ExecEventComplete(ExecEventComplete),
    AuditEvent(AuditEvent),
    DnsEvent(DnsEvent),
    SubstitutionEvent(SubstitutionEvent),
    SecurityRuleEvent(SecurityRuleEvent),
    SecurityAskEvent(SecurityAskEvent),
    SecurityDecisionEvent(SecurityDecisionEvent),
    PolicyMutationEvent(PolicyMutationEvent),
    /// What the host did to a session or itself; hash-chained per ledger.
    HostEvent(HostEvent),
    /// Registry rows of a network database; upserted by key, disk-only.
    Network(NetworkRecord),
    NetworkMembership(NetworkMembership),
    NetworkDeparture(NetworkDeparture),
}

/// What a flush barrier reports back: `Err` when the disk flush the barrier
/// forced did not happen, so a caller relying on cross-process visibility
/// (an external reader syncing from disk) is not told the rows are there.
type FlushOutcome = Result<(), String>;

/// What one retention request reports back.
type RetainReply = tokio::sync::oneshot::Sender<Result<RetainOutcome, String>>;

/// What the writer thread is asked to do. An enum rather than a pair of
/// options: every payload combination it can hold is now one the loop has to
/// name, which is how a third kind of request arrives without an
/// `unreachable!` standing between it and the code that runs it.
// A ledger event is a few hundred bytes and a barrier is a handful. Boxing
// the write to even them out would put a heap allocation and a copy on the
// path every telemetry event takes, to save moving bytes that the old
// `Option<WriteOp>` field moved anyway.
//
// The size is asserted below rather than stated here, because a justification
// that rests on a number nobody rechecks is how an enum quietly grows into
// something the reasoning no longer covers.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
enum WriterMessage {
    Write(WriteEnvelope),
    /// A producer batch that must reach the SQLite transaction as one unit.
    /// Keeping the vector intact avoids rebuilding a protocol batch through
    /// one channel send and one scheduler wakeup per row.
    WriteBatch(Vec<WriteEnvelope>),
    /// Commit everything queued before this and report whether the disk flush
    /// happened.
    Flush(tokio::sync::oneshot::Sender<FlushOutcome>),
    /// Drop archived bodies written before this RFC 3339 cutoff. A barrier
    /// like `Flush`, because it needs the queue committed before it runs.
    Retain {
        cutoff: String,
        reply: RetainReply,
    },
}

impl WriterMessage {
    fn write(op: WriteOp) -> Self {
        Self::Write(WriteEnvelope { op, commitment: None })
    }

    fn committed(op: WriteOp, commitment: capsem_proto::ledger_commitment::LedgerCommitment) -> Self {
        Self::Write(WriteEnvelope {
            op,
            commitment: Some(commitment),
        })
    }

    fn committed_batch(writes: Vec<(WriteOp, capsem_proto::ledger_commitment::LedgerCommitment)>) -> Self {
        Self::WriteBatch(
            writes
                .into_iter()
                .map(|(op, commitment)| WriteEnvelope {
                    op,
                    commitment: Some(commitment),
                })
                .collect(),
        )
    }

    fn flush(reply: tokio::sync::oneshot::Sender<FlushOutcome>) -> Self {
        Self::Flush(reply)
    }

    fn event_count(&self) -> usize {
        match self {
            Self::Write(_) => 1,
            Self::WriteBatch(writes) => writes.len(),
            Self::Flush(_) | Self::Retain { .. } => 0,
        }
    }
}

#[derive(Debug)]
struct WriteEnvelope {
    op: WriteOp,
    commitment: Option<capsem_proto::ledger_commitment::LedgerCommitment>,
}

impl WriteEnvelope {
    #[cfg(test)]
    fn plain(op: WriteOp) -> Self {
        Self { op, commitment: None }
    }
}

/// What the argument above assumes. Not a budget anyone chose -- it is the
/// size at which "moving this is cheaper than boxing it" was weighed, and a
/// message that outgrows it by a lot deserves the question asked again.
const _: () = assert!(std::mem::size_of::<WriterMessage>() <= 1024);

#[derive(Clone)]
struct WriterSender {
    tx: mpsc::SyncSender<QueuedWriterMessage>,
    budget: Arc<WriterQueueBudget>,
}

type WriterReceiver = mpsc::Receiver<QueuedWriterMessage>;

struct QueuedWriterMessage {
    message: WriterMessage,
    _permit: WriterQueuePermit,
}

impl QueuedWriterMessage {
    fn into_message(self) -> WriterMessage {
        self.message
    }
}

struct WriterQueueBudget {
    capacity: usize,
    used: AtomicUsize,
}

struct WriterQueuePermit {
    budget: Arc<WriterQueueBudget>,
    weight: usize,
}

impl Drop for WriterQueuePermit {
    fn drop(&mut self) {
        self.budget.used.fetch_sub(self.weight, Ordering::Release);
    }
}

impl WriterQueueBudget {
    fn try_reserve(self: &Arc<Self>, weight: usize) -> Option<WriterQueuePermit> {
        let mut used = self.used.load(Ordering::Acquire);
        loop {
            let next = used.checked_add(weight)?;
            if next > self.capacity {
                return None;
            }
            match self
                .used
                .compare_exchange_weak(used, next, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => {
                    return Some(WriterQueuePermit {
                        budget: Arc::clone(self),
                        weight,
                    })
                }
                Err(current) => used = current,
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum WriterTrySendError {
    Full,
    Disconnected,
}

impl WriterSender {
    fn try_send(&self, message: &mut Option<WriterMessage>) -> Result<(), WriterTrySendError> {
        let weight = message.as_ref().expect("writer message is present").event_count();
        let Some(permit) = self.budget.try_reserve(weight) else {
            return Err(WriterTrySendError::Full);
        };
        match self.tx.try_send(QueuedWriterMessage {
            message: message.take().expect("writer message is present"),
            _permit: permit,
        }) {
            Ok(()) => Ok(()),
            Err(mpsc::TrySendError::Full(queued)) => {
                *message = Some(queued.into_message());
                Err(WriterTrySendError::Full)
            }
            Err(mpsc::TrySendError::Disconnected(queued)) => {
                *message = Some(queued.into_message());
                Err(WriterTrySendError::Disconnected)
            }
        }
    }

    fn send(&self, message: WriterMessage) -> Result<(), &'static str> {
        let mut message = Some(message);
        let mut wait = Duration::from_micros(50);
        loop {
            match self.try_send(&mut message) {
                Ok(()) => return Ok(()),
                Err(WriterTrySendError::Full) => {
                    std::thread::sleep(wait);
                    wait = (wait * 2).min(Duration::from_millis(5));
                }
                Err(WriterTrySendError::Disconnected) => return Err("db writer channel closed"),
            }
        }
    }
}

fn writer_channel(capacity: usize) -> (WriterSender, WriterReceiver) {
    let capacity = capacity.max(1);
    let (tx, rx) = mpsc::sync_channel(capacity);
    (
        WriterSender {
            tx,
            budget: Arc::new(WriterQueueBudget {
                capacity,
                used: AtomicUsize::new(0),
            }),
        },
        rx,
    )
}

mod barriers;
mod batch;
mod operation;
pub(crate) use operation::commitment_event_fingerprint;
pub use operation::commitment_event_hash;
mod recording;
mod remote;
#[cfg(test)]
pub(crate) use remote::test_commitment_channel;
mod retention;
mod retention_faults;
mod worker;
mod writer_lock;
#[cfg(test)]
pub(crate) use retention_faults::{fail_retention_for_path_for_tests, RetentionFault};
use worker::writer_loop;

use barriers::Barriers;
use batch::{execute_memory_batch, retry_batch_ops_individually};
use operation::{affected_memory_tables, write_op_affects_storage};
use recording::{batch_size_bucket, record_batch, record_enqueue};
pub use retention::RetainOutcome;

/// A dedicated writer thread that owns the SQLite connection.
///
/// Callers send `WriteOp` values through an mpsc channel. The writer thread
/// blocks until ops arrive, drains the queue, and executes them in a single
/// transaction for efficiency.
///
/// Shutdown is explicit-cleanup safe via `shutdown_blocking(&self)`: callers
/// holding an `Arc<DbWriter>` can deterministically drop the stored sender
/// and join the writer thread without waiting for `Drop` to run when the
/// last Arc clone disappears. This matters under the 1s SIGTERM-to-SIGKILL
/// budget that the service enforces on `capsem-process` teardown -- see
/// /dev-rust-patterns "Signal-driven explicit cleanup".
pub struct DbWriter {
    /// Stored sender. `shutdown_blocking` takes it out; `write` clones it
    /// under the lock and releases the lock before touching the producer
    /// channel so hot-path latency is unaffected.
    tx: std::sync::Mutex<Option<WriterSender>>,
    join_handle: std::sync::Mutex<Option<std::thread::JoinHandle<()>>>,
    db_path: PathBuf,
    /// Raw bytes the writer thread has staged but not yet written to the archive.
    /// Published by the writer thread after every batch so a test can prove
    /// the bound without a second view of the thread's state.
    pending_body_bytes: Arc<AtomicU64>,
    remote: Option<remote::RemoteWriter>,
}

pub(crate) struct PreparedDbWriter {
    tx: WriterSender,
    rx: WriterReceiver,
    held: capsem_foundation::unix::lock::FileLock,
    conn: Connection,
    db_path: PathBuf,
    batch_capacity: usize,
    pending_body_bytes: Arc<AtomicU64>,
    bodies: BodyArchive,
    tally: LedgerTally,
}

impl DbWriter {
    /// Spawn a dedicated writer thread that owns the DB connection.
    /// `capacity` controls the mpsc channel size (backpressure).
    pub fn open(path: &Path, capacity: usize) -> rusqlite::Result<Self> {
        Self::open_with_codecs(path, capacity, ArchiveCodecs::default())
    }

    pub fn open_with_codecs(path: &Path, capacity: usize, codecs: ArchiveCodecs) -> rusqlite::Result<Self> {
        Self::prepare_with_clock_and_codecs(path, capacity, SystemTime::now, codecs).map(PreparedDbWriter::start)
    }

    /// `open`, with the archive index's timestamps read from `now`.
    ///
    /// Only a replay of an existing ledger has any business supplying one; see
    /// `LedgerClock`.
    pub fn open_with_clock(path: &Path, capacity: usize, now: LedgerClock) -> rusqlite::Result<Self> {
        Self::prepare_with_clock_and_codecs(path, capacity, now, ArchiveCodecs::default()).map(PreparedDbWriter::start)
    }

    pub(crate) fn prepare_with_capacities(
        path: &Path,
        channel_capacity: usize,
        batch_capacity: usize,
        codecs: ArchiveCodecs,
    ) -> rusqlite::Result<PreparedDbWriter> {
        Self::prepare_with_clock_and_capacities(path, channel_capacity, batch_capacity, SystemTime::now, codecs)
    }

    fn prepare_with_clock_and_codecs(
        path: &Path,
        capacity: usize,
        now: LedgerClock,
        codecs: ArchiveCodecs,
    ) -> rusqlite::Result<PreparedDbWriter> {
        Self::prepare_with_clock_and_capacities(path, capacity, capacity, now, codecs)
    }

    fn prepare_with_clock_and_capacities(
        path: &Path,
        channel_capacity: usize,
        batch_capacity: usize,
        now: LedgerClock,
        codecs: ArchiveCodecs,
    ) -> rusqlite::Result<PreparedDbWriter> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let mut last_busy = None;
        for _ in 0..50 {
            match Self::prepare_once(path, channel_capacity, batch_capacity, now, codecs.clone()) {
                Ok(prepared) => return Ok(prepared),
                Err(error) if is_sqlite_busy(&error) => {
                    last_busy = Some(error);
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(error) => return Err(error),
            }
        }
        Err(last_busy.unwrap_or(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(ErrorCode::DatabaseBusy as i32),
            Some("database remained busy while opening writer".to_string()),
        )))
    }

    fn prepare_once(
        path: &Path,
        channel_capacity: usize,
        batch_capacity: usize,
        now: LedgerClock,
        codecs: ArchiveCodecs,
    ) -> rusqlite::Result<PreparedDbWriter> {
        let held = writer_lock::acquire(path)?;
        legacy::set_aside_pre_archive_ledger(path)?;
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_URI;
        let conn = Connection::open_with_flags(path, flags)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        schema::apply_pragmas(&conn)?;
        // One statement per table per target; the default 16 would evict.
        conn.set_prepared_statement_cache_capacity(capsem_proto::CACHE_SQLITE_PREPARED_STATEMENTS);
        schema::record_sqlite_mmap_telemetry(&conn, path, "writer", "open");
        let bodies = match schema::archive_schema_status(&conn)? {
            schema::ArchiveSchemaStatus::Fresh => {
                let (bodies, archive_lock, header) = BodyArchive::prepare_new(path, now, codecs)?;
                schema::create_tables_with_archive_header(&conn, Some(header))?;
                drop(archive_lock);
                bodies
            }
            schema::ArchiveSchemaStatus::Current(_) => {
                schema::create_tables(&conn)?;
                BodyArchive::open_existing(path, now, &conn, codecs)?
            }
            // Moved aside above, under this same writer lock.
            schema::ArchiveSchemaStatus::PreArchive => {
                return Err(rusqlite::Error::InvalidParameterName(
                    schema::PRE_ARCHIVE_REFUSAL.to_string(),
                ))
            }
        };
        let memory_uri = schema::memory_uri_for_path(path);
        schema::with_memory_schema_lock(|| {
            schema::create_memory_tables(&conn, &memory_uri)?;
            schema::seed_memory_sequences(&conn, schema::hot_ledger_tables())
        })?;
        // Counting resumes from what the ledger last committed, never from a
        // scan of its rows.
        let tally = LedgerTally::restored(counters::load(&conn)?);

        let batch_capacity = if batch_capacity == 0 {
            DEFAULT_BATCH_CAPACITY
        } else {
            batch_capacity
        };
        let (tx, rx) = writer_channel(channel_capacity);
        let db_path = path.to_path_buf();
        let pending_body_bytes = Arc::new(AtomicU64::new(0));
        Ok(PreparedDbWriter {
            tx,
            rx,
            held,
            conn,
            db_path,
            batch_capacity,
            pending_body_bytes,
            bodies,
            tally,
        })
    }

    /// Open an in-memory database (for testing).
    pub fn open_in_memory(capacity: usize) -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        schema::apply_pragmas(&conn)?;
        // One statement per table per target; the default 16 would evict.
        conn.set_prepared_statement_cache_capacity(capsem_proto::CACHE_SQLITE_PREPARED_STATEMENTS);
        schema::create_tables(&conn)?;
        let bodies = BodyArchive::disabled(SystemTime::now);
        let memory_uri = schema::memory_uri_for_name(&format!(
            "writer-open-in-memory-{}-{}",
            std::process::id(),
            IN_MEMORY_WRITER_ID.fetch_add(1, Ordering::Relaxed)
        ));
        schema::with_memory_schema_lock(|| {
            schema::create_memory_tables(&conn, &memory_uri)?;
            schema::seed_memory_sequences(&conn, schema::hot_ledger_tables())
        })?;
        let tally = LedgerTally::restored(counters::load(&conn)?);

        let schedule = WriterSchedule::new(capacity);
        let (tx, rx) = writer_channel(schedule.capacity);
        let pending_body_bytes = Arc::new(AtomicU64::new(0));
        let loop_pending_body_bytes = Arc::clone(&pending_body_bytes);
        let join_handle = std::thread::Builder::new()
            .name("capsem-db-writer".into())
            .spawn(move || writer_loop(conn, rx, None, schedule, &loop_pending_body_bytes, bodies, tally))
            .expect("failed to spawn db writer thread");

        Ok(Self {
            tx: std::sync::Mutex::new(Some(tx)),
            join_handle: std::sync::Mutex::new(Some(join_handle)),
            db_path: PathBuf::from(":memory:"),
            pending_body_bytes,
            remote: None,
        })
    }

    /// Construct the familiar producer facade over a coordinator-minted
    /// channel. This process retains no SQLite connection or writer lock; the
    /// dedicated ledger worker owns both.
    pub fn from_ledger_channel(
        stream: std::os::unix::net::UnixStream,
        commitment_stream: std::os::unix::net::UnixStream,
        grant: capsem_proto::ledger::LedgerChannelGrant,
        logical_path: &Path,
        capacity: usize,
    ) -> std::io::Result<Self> {
        Ok(Self {
            tx: std::sync::Mutex::new(None),
            join_handle: std::sync::Mutex::new(None),
            db_path: logical_path.to_path_buf(),
            pending_body_bytes: Arc::new(AtomicU64::new(0)),
            remote: Some(remote::RemoteWriter::start(stream, commitment_stream, grant, capacity)?),
        })
    }

    /// Wait until the writer thread has committed every operation enqueued
    /// before this barrier. This is non-destructive: unlike shutdown, it keeps
    /// the writer alive for future events.
    pub async fn flush(&self) {
        if let Err(error) = self.flush_checked().await {
            warn!(error = %error, "db flush barrier did not complete");
        }
    }

    /// `flush`, reporting whether the disk flush the barrier forced happened.
    /// Every reader reads the file, so an `Err` means no reader will see the
    /// rows yet, and the caller must not claim otherwise.
    pub async fn flush_checked(&self) -> Result<(), String> {
        if let Some(remote) = &self.remote {
            return remote.flush().await;
        }
        let Some(tx) = self.clone_sender() else {
            return Ok(());
        };
        let (reply, rx) = tokio::sync::oneshot::channel();
        send_with_backpressure(&tx, WriterMessage::flush(reply))
            .await
            .map_err(|e| format!("db writer channel closed, dropping flush barrier: {e}"))?;
        rx.await
            .map_err(|e| format!("db writer flush barrier dropped before ack: {e}"))?
    }

    /// Drop archived bodies whose blocks were last written before `cutoff` (RFC 3339).
    ///
    /// A barrier, like `flush_checked`: everything queued before this call is
    /// committed first, so a body written a moment ago is either indexed and
    /// judged by the cutoff or not yet written at all -- never dropped
    /// because its index row had not landed.
    ///
    /// The work happens on the writer thread because that thread owns both
    /// halves of the ledger. A caller that holds no writer holds no right to
    /// rewrite either one.
    pub async fn retain_bodies_since(&self, cutoff: &str) -> Result<RetainOutcome, String> {
        if let Some(remote) = &self.remote {
            return remote.retain(cutoff).await;
        }
        let Some(tx) = self.clone_sender() else {
            return Err("db writer is shut down; nothing was retained".to_string());
        };
        let (reply, rx) = tokio::sync::oneshot::channel();
        send_with_backpressure(
            &tx,
            WriterMessage::Retain {
                cutoff: cutoff.to_string(),
                reply,
            },
        )
        .await
        .map_err(|error| format!("db writer channel closed, dropping retention request: {error}"))?;
        rx.await
            .map_err(|error| format!("db writer dropped the retention request before answering: {error}"))?
    }

    /// Wait for short-lived producers to enqueue their final rows, then flush
    /// the writer queue. Use at external command boundaries where the guest
    /// process can exit a few milliseconds before host-side socket closeout
    /// telemetry has finished enqueueing its ledger rows.
    pub async fn flush_after_quiescence(&self, settle: std::time::Duration) {
        if !settle.is_zero() {
            tokio::time::sleep(settle).await;
        }
        self.flush().await;
    }

    /// Deterministically shut down the writer thread: drop the stored
    /// sender and join. Safe to call through a shared `Arc<DbWriter>` --
    /// other Arc clones stay valid but subsequent `write` calls become
    /// no-ops. Idempotent. Blocks until the writer thread drains its queue
    /// and runs the final `PRAGMA wal_checkpoint(TRUNCATE)`. Call from a
    /// blocking thread (e.g. via `tokio::task::spawn_blocking`).
    pub fn shutdown_blocking(&self) {
        if let Some(remote) = &self.remote {
            remote.shutdown();
            return;
        }
        let _ = self.tx.lock().unwrap().take();
        let handle = self.join_handle.lock().unwrap().take();
        if let Some(handle) = handle {
            let _ = handle.join();
        }
    }

    /// Open a read-only connection to the same DB file (WAL concurrent reader).
    /// It sees what the writer has flushed, not what it has only accepted.
    /// Returns Err for in-memory writers (no file to share between connections).
    pub fn reader(&self) -> rusqlite::Result<crate::reader::DbReader> {
        if self.remote.is_some() {
            return Err(rusqlite::Error::InvalidPath(self.db_path.clone()));
        }
        if self.db_path.to_str() == Some(":memory:") {
            return Err(rusqlite::Error::InvalidPath(self.db_path.clone()));
        }
        crate::reader::DbReader::open(&self.db_path)
    }

    /// The path to the database file.
    pub fn path(&self) -> &Path {
        &self.db_path
    }

    /// Raw body bytes the writer thread has staged into the archive's open
    /// block but not yet written. They reach `session.bodies` at the next
    /// disk flush or when the block closes, so this is the backlog a crash would lose.
    pub fn pending_body_bytes(&self) -> u64 {
        if self.remote.is_some() {
            return 0;
        }
        self.pending_body_bytes.load(Ordering::Acquire)
    }
}

impl PreparedDbWriter {
    pub(crate) fn start(self) -> DbWriter {
        let Self {
            tx,
            rx,
            held,
            conn,
            db_path,
            batch_capacity,
            pending_body_bytes,
            bodies,
            tally,
        } = self;
        let writer_loop_db_path = Some(db_path.clone());
        let loop_pending_body_bytes = Arc::clone(&pending_body_bytes);
        let join_handle = std::thread::Builder::new()
            .name("capsem-db-writer".into())
            .spawn(move || {
                let _held = held;
                writer_loop(
                    conn,
                    rx,
                    writer_loop_db_path,
                    batch_capacity,
                    &loop_pending_body_bytes,
                    bodies,
                    tally,
                )
            })
            .expect("failed to spawn db writer thread");
        DbWriter {
            tx: std::sync::Mutex::new(Some(tx)),
            join_handle: std::sync::Mutex::new(Some(join_handle)),
            db_path,
            pending_body_bytes,
            remote: None,
        }
    }
}

impl Drop for DbWriter {
    fn drop(&mut self) {
        self.shutdown_blocking();
    }
}

fn is_sqlite_busy(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(inner, _)
            if matches!(inner.code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
    )
}

/// Longest pause between attempts to queue a message while the writer is full.
const BACKPRESSURE_MAX_WAIT: Duration = Duration::from_millis(5);

/// Queue a message, waiting for room.
///
/// The channel is a std `sync_channel` drained by the writer thread, so there
/// is nothing async to await for space. Yield once, then sleep with doubling
/// backoff. Retrying immediately after `yield_now` pinned every producer task
/// at full CPU for as long as the writer thread was inside a disk flush, which
/// can be seconds with a large dirty set or a busy-timeout on the file.
async fn send_with_backpressure(tx: &WriterSender, message: WriterMessage) -> Result<(), String> {
    let mut message = Some(message);
    let mut wait = Duration::from_micros(50);
    let mut attempts = 0u32;
    loop {
        match tx.try_send(&mut message) {
            Ok(()) => return Ok(()),
            Err(WriterTrySendError::Full) => {
                if attempts == 0 {
                    tokio::task::yield_now().await;
                } else {
                    tokio::time::sleep(wait).await;
                    wait = (wait * 2).min(BACKPRESSURE_MAX_WAIT);
                }
                attempts += 1;
            }
            Err(WriterTrySendError::Disconnected) => {
                return Err("db writer channel closed".to_string());
            }
        }
    }
}

/// The writer thread loop: block-then-drain batching.
#[derive(Clone, Copy)]
enum WriteTarget {
    Memory,
}

impl WriteTarget {
    fn table(self, name: &str) -> String {
        match self {
            WriteTarget::Memory if !schema::is_disk_only_table(name) => format!("mem.{name}"),
            WriteTarget::Memory => format!("main.{name}"),
        }
    }
}

/// Execute through the connection's prepared-statement cache.
///
/// Every insert here is one of a small fixed set of statements (one per
/// table, per memory or disk target). `Connection::execute` parsed and
/// planned that SQL again for every row, and on a busy proxy the parser
/// showed up in the writer thread's profile next to the inserts themselves.
fn execute_cached(conn: &Connection, sql: &str, params: impl rusqlite::Params) -> rusqlite::Result<usize> {
    conn.prepare_cached(sql)?.execute(params)
}

mod event_rows;
mod traffic_rows;
use event_rows::{
    insert_audit_event, insert_dns_event, insert_host_event, insert_policy_mutation_event, insert_security_ask_event,
    insert_security_decision_event, insert_security_rule_event, insert_substitution_event,
};
use traffic_rows::{insert_exec_event, insert_file_event, insert_mcp_call, insert_net_event, update_exec_event};

#[cfg(test)]
mod tests;
