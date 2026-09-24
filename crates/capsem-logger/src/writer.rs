use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant, SystemTime};

use rusqlite::{params, Connection, ErrorCode, OpenFlags, OptionalExtension};
use tracing::{error, warn};
use uuid::Uuid;

use crate::counters::{self, LedgerCounters, LedgerTally};
use crate::events::{
    AuditEvent, DnsEvent, ExecEvent, ExecEventComplete, FileEvent, McpCall, ModelCall, NetEvent, NetworkMembership,
    NetworkRecord, ProfileMutationEvent, SecurityAskEvent, SecurityDecisionEvent, SecurityRuleEvent, SubstitutionEvent,
    TransportEvent,
};
use crate::schema;

mod bodies;
mod flush_faults;
mod model_rows;
mod producer;
#[cfg(test)]
pub(crate) use bodies::close_blocks_at_every_flush_for_tests;
pub(crate) use bodies::{archive_lock_path_for_db, archive_path_for_db};
use bodies::{BodyArchive, EventBodyBlob};
use flush_faults::take_disk_flush_failure_for_tests;
#[cfg(test)]
pub(crate) use flush_faults::{fail_disk_flushes_for_path_for_tests, fail_disk_flushes_for_tests};
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
pub(crate) const MAX_BODY_BLOB_BYTES: usize = 10 * 1024 * 1024;
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
const DISK_FLUSH_THRESHOLD_OPS: usize = 1_000_000;
const DISK_FLUSH_INTERVAL: Duration = Duration::from_secs(5);

pub const DB_ENQUEUE_SPAN: &str = "capsem.db.enqueue";
pub const DB_WRITE_BATCH_SPAN: &str = "capsem.db.write_batch";
pub const DB_SHUTDOWN_FLUSH_SPAN: &str = "capsem.db.shutdown_flush";

pub const DB_ENQUEUE_WAIT_MS: &str = "db.enqueue_wait_ms";
pub const DB_ENQUEUE_TOTAL: &str = "db.enqueue_total";
pub const DB_WRITE_BATCH_TOTAL: &str = "db.write_batch_total";
pub const DB_WRITE_BATCH_DURATION_MS: &str = "db.write_batch_duration_ms";
pub const DB_WRITE_OP_REJECTED_TOTAL: &str = "db.write_op_rejected_total";
pub const DB_WRITE_BATCH_SIZE: &str = "db.write_batch_size";
pub const DB_WRITE_BATCH_CAPACITY: &str = "db.write_batch_capacity";
pub const DB_WRITE_BATCH_ROWS_PER_SEC: &str = "db.write_batch_rows_per_sec";
pub const DB_WRITE_OPS_TOTAL: &str = "db.write_ops_total";
pub const DB_SHUTDOWN_FLUSH_MS: &str = "db.shutdown_flush_ms";
/// Bodies the archive gave up on, by the step that gave up: the only place a
/// poisoned archive surfaces besides a log line.
pub const DB_ARCHIVE_BODIES_DROPPED_TOTAL: &str = "db.archive_bodies_dropped_total";
/// Bodies indexed against identical bytes already stored, labelled by
/// scope: `block` (the open block) or `archive` (a committed earlier segment).
pub const DB_ARCHIVE_BODIES_DEDUPLICATED_TOTAL: &str = "db.archive_bodies_deduplicated_total";
/// Ops the writer holds in memory waiting for a disk flush. It falls to zero
/// on every flush that lands; a value that only climbs is a disk the writer
/// cannot flush to, with the session's rows piling up in RAM.
pub const DB_MEMORY_UNFLUSHED_OPS: &str = "db.memory_unflushed_ops";

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
    let value = Uuid::new_v4().simple().to_string();
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
    let bytes = body.filter(|bytes| !bytes.is_empty())?;
    let head = &bytes[..bytes.len().min(PREVIEW_BYTES)];
    cap_bytes(&Some(String::from_utf8_lossy(head).into_owned()), PREVIEW_BYTES)
}

fn blake3_ref(value: &str) -> String {
    format!("blake3:{}", blake3::hash(value.as_bytes()).to_hex())
}

fn blake3_bytes_ref(value: &[u8]) -> String {
    format!("blake3:{}", blake3::hash(value).to_hex())
}

/// Typed write operations sent to the writer thread.
#[derive(Debug, Clone)]
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
    ProfileMutationEvent(ProfileMutationEvent),
    /// Registry rows of a network database; upserted by key, disk-only.
    Network(NetworkRecord),
    NetworkMembership(NetworkMembership),
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
    Write(WriteOp),
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
        Self::Write(op)
    }

    fn flush(reply: tokio::sync::oneshot::Sender<FlushOutcome>) -> Self {
        Self::Flush(reply)
    }
}

/// What the argument above assumes. Not a budget anyone chose -- it is the
/// size at which "moving this is cheaper than boxing it" was weighed, and a
/// message that outgrows it by a lot deserves the question asked again.
const _: () = assert!(std::mem::size_of::<WriterMessage>() <= 1024);

type WriterSender = mpsc::SyncSender<WriterMessage>;

fn writer_channel(capacity: usize) -> (WriterSender, mpsc::Receiver<WriterMessage>) {
    mpsc::sync_channel(capacity.max(1))
}

mod barriers;
mod batch;
mod operation;
mod recording;
mod retention;
mod retention_faults;
mod writer_lock;
#[cfg(test)]
pub(crate) use retention_faults::{fail_retention_for_path_for_tests, RetentionFault};

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
}

impl DbWriter {
    /// Spawn a dedicated writer thread that owns the DB connection.
    /// `capacity` controls the mpsc channel size (backpressure).
    pub fn open(path: &Path, capacity: usize) -> rusqlite::Result<Self> {
        Self::open_with_clock(path, capacity, SystemTime::now)
    }

    /// `open`, with the archive index's timestamps read from `now`.
    ///
    /// Only a replay of an existing ledger has any business supplying one; see
    /// `LedgerClock`.
    pub fn open_with_clock(path: &Path, capacity: usize, now: LedgerClock) -> rusqlite::Result<Self> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let mut last_busy = None;
        for _ in 0..50 {
            match Self::open_once(path, capacity, now) {
                Ok(writer) => return Ok(writer),
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

    fn open_once(path: &Path, capacity: usize, now: LedgerClock) -> rusqlite::Result<Self> {
        let held = writer_lock::acquire(path)?;
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_URI;
        let conn = Connection::open_with_flags(path, flags)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        schema::apply_pragmas(&conn)?;
        // One statement per table per target; the default 16 would evict.
        conn.set_prepared_statement_cache_capacity(64);
        schema::record_sqlite_mmap_telemetry(&conn, path, "writer", "open");
        let bodies = match schema::archive_schema_status(&conn)? {
            schema::ArchiveSchemaStatus::Fresh => {
                let (bodies, archive_lock, header) = BodyArchive::prepare_new(path, now)?;
                schema::create_tables_with_archive_header(&conn, Some(header))?;
                drop(archive_lock);
                bodies
            }
            schema::ArchiveSchemaStatus::Current(_) => {
                schema::create_tables(&conn)?;
                BodyArchive::open_existing(path, now, &conn)?
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

        let batch_capacity = if capacity == 0 {
            DEFAULT_BATCH_CAPACITY
        } else {
            capacity
        };
        let (tx, rx) = writer_channel(batch_capacity);
        let db_path = path.to_path_buf();
        let writer_loop_db_path = Some(db_path.clone());
        let pending_body_bytes = Arc::new(AtomicU64::new(0));
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

        Ok(Self {
            tx: std::sync::Mutex::new(Some(tx)),
            join_handle: std::sync::Mutex::new(Some(join_handle)),
            db_path,
            pending_body_bytes,
        })
    }

    /// Open an in-memory database (for testing).
    pub fn open_in_memory(capacity: usize) -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        schema::apply_pragmas(&conn)?;
        // One statement per table per target; the default 16 would evict.
        conn.set_prepared_statement_cache_capacity(64);
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

        let batch_capacity = if capacity == 0 {
            DEFAULT_BATCH_CAPACITY
        } else {
            capacity
        };
        let (tx, rx) = writer_channel(batch_capacity);
        let pending_body_bytes = Arc::new(AtomicU64::new(0));
        let loop_pending_body_bytes = Arc::clone(&pending_body_bytes);
        let join_handle = std::thread::Builder::new()
            .name("capsem-db-writer".into())
            .spawn(move || writer_loop(conn, rx, None, batch_capacity, &loop_pending_body_bytes, bodies, tally))
            .expect("failed to spawn db writer thread");

        Ok(Self {
            tx: std::sync::Mutex::new(Some(tx)),
            join_handle: std::sync::Mutex::new(Some(join_handle)),
            db_path: PathBuf::from(":memory:"),
            pending_body_bytes,
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
        self.pending_body_bytes.load(Ordering::Acquire)
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
async fn send_with_backpressure(tx: &WriterSender, mut message: WriterMessage) -> Result<(), String> {
    let mut wait = Duration::from_micros(50);
    let mut attempts = 0u32;
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
                return Err("db writer channel closed".to_string());
            }
        }
    }
}

/// The writer thread loop: block-then-drain batching.
fn writer_loop(
    conn: Connection,
    rx: mpsc::Receiver<WriterMessage>,
    db_path: Option<PathBuf>,
    batch_capacity: usize,
    pending_body_bytes: &AtomicU64,
    mut bodies: BodyArchive,
    mut tally: LedgerTally,
) {
    let mut flush_watermarks =
        schema::with_memory_schema_lock(|| schema::initial_memory_flush_watermarks(&conn, schema::hot_ledger_tables()))
            .unwrap_or_else(|error| {
                warn!(error = %error, "db initial memory flush watermark load failed");
                schema::MemoryFlushWatermarks::new()
            });
    // Rows at or below the open-time watermark were written by an earlier
    // writer; exec ids restart with the process, so a completion must not
    // reach one of those.
    let exec_floor = flush_watermarks.get("exec_events").copied().unwrap_or(0);
    let mut dirty_tables = BTreeSet::new();
    let mut dirty_ops = 0_usize;
    let mut last_disk_flush = Instant::now();
    // 1. Block until at least one op arrives. Returns None when all
    //    Senders are dropped (clean shutdown) and ends the loop.
    loop {
        let first_message = if dirty_ops == 0 {
            rx.recv().ok()
        } else {
            match rx.recv_timeout(DISK_FLUSH_INTERVAL) {
                Ok(message) => Some(message),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if let Err(error) = flush_dirty_tables_to_disk(
                        &conn,
                        &mut dirty_tables,
                        &mut flush_watermarks,
                        db_path.as_deref(),
                        &mut bodies,
                        tally.counters(),
                    ) {
                        warn!(error = %error, "db interval flush failed");
                    } else {
                        dirty_ops = 0;
                        last_disk_flush = Instant::now();
                    }
                    ::metrics::gauge!(DB_MEMORY_UNFLUSHED_OPS).set(dirty_ops as f64);
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => None,
            }
        };
        let Some(first_message) = first_message else {
            break;
        };

        let mut batch = Vec::with_capacity(batch_capacity);
        let mut barriers = Barriers::default();
        let mut at_barrier = barriers.accept(first_message, &mut batch);

        // 2. Drain any ops already queued (non-blocking).
        while !at_barrier && batch.len() < batch_capacity {
            match rx.try_recv() {
                Ok(message) => at_barrier = barriers.accept(message, &mut batch),
                Err(_) => break,
            }
        }

        // 3. Execute entire batch in a single transaction.
        let batch_size = batch.len();
        let batch_bucket = batch_size_bucket(batch_size);
        let span = tracing::debug_span!(
            target: "capsem.db",
            DB_WRITE_BATCH_SPAN,
            batch_size_bucket = batch_bucket,
            status = tracing::field::Empty,
        );
        let started = Instant::now();
        let batch_capacity = batch.capacity();
        if batch.is_empty() {
            record_batch(started, batch_size, batch_capacity, batch_bucket, "ok", &span);
        } else {
            match span.in_scope(|| execute_memory_batch(&conn, &batch, &mut bodies, exec_floor, &mut tally)) {
                Ok(outcome) => {
                    dirty_tables.extend(outcome.tables);
                    dirty_ops += outcome.written;
                    record_batch(started, batch_size, batch_capacity, batch_bucket, "ok", &span);
                }
                Err(e) => {
                    record_batch(started, batch_size, batch_capacity, batch_bucket, "error", &span);
                    warn!(
                        error = %e,
                        count = batch.len(),
                        "db memory write batch failed; retrying its ops individually"
                    );
                    let salvaged = span
                        .in_scope(|| retry_batch_ops_individually(&conn, &batch, &mut bodies, exec_floor, &mut tally));
                    dirty_tables.extend(salvaged.tables);
                    dirty_ops += salvaged.written;
                }
            }
        }
        // A burst of large bodies closes its block as soon as the batch ends
        // rather than growing it until the interval expires. Its index rows
        // wait for the next transaction; its bytes are already on disk.
        bodies.close_if_due();
        pending_body_bytes.store(bodies.pending_bytes() as u64, Ordering::Release);
        let disk_flush_due = dirty_ops >= DISK_FLUSH_THRESHOLD_OPS
            || last_disk_flush.elapsed() >= DISK_FLUSH_INTERVAL
            || barriers.waiting();
        let mut barrier_outcome: FlushOutcome = Ok(());
        if disk_flush_due {
            match flush_dirty_tables_to_disk(
                &conn,
                &mut dirty_tables,
                &mut flush_watermarks,
                db_path.as_deref(),
                &mut bodies,
                tally.counters(),
            ) {
                Ok(()) => {
                    dirty_ops = 0;
                    last_disk_flush = Instant::now();
                    pending_body_bytes.store(bodies.pending_bytes() as u64, Ordering::Release);
                }
                Err(error) => {
                    warn!(error = %error, "db dirty table flush failed");
                    barrier_outcome = Err(format!("db dirty table flush failed: {error}"));
                }
            }
        }
        ::metrics::gauge!(DB_MEMORY_UNFLUSHED_OPS).set(dirty_ops as f64);
        barriers.answer(&conn, &mut bodies, &barrier_outcome);
    }

    // Test hook: lets `test_wal_absent_after_clean_shutdown`-style tests
    // simulate a slow checkpoint so the explicit-cleanup path can be
    // distinguished from implicit tokio-runtime-drop ordering. Gated on
    // an env var so it's a no-op in production.
    if let Ok(ms) = std::env::var("CAPSEM_TEST_SLOW_CHECKPOINT_MS") {
        if let Ok(ms) = ms.parse::<u64>() {
            std::thread::sleep(std::time::Duration::from_millis(ms));
        }
    }

    // The open block ends with its FINAL segment, committed by this flush.
    bodies.close_block();
    if let Err(error) = flush_dirty_tables_to_disk(
        &conn,
        &mut dirty_tables,
        &mut flush_watermarks,
        db_path.as_deref(),
        &mut bodies,
        tally.counters(),
    ) {
        warn!(error = %error, "db shutdown dirty table flush failed");
        // There is no next flush to retry into, so the bodies still waiting
        // for an index row are lost here. Counted, so the drop counter closes
        // over the session rather than ending on a number that is short by
        // however much the last flush was carrying.
        bodies.abandon_uncommitted("shutdown");
    }
    bodies.sync();
    pending_body_bytes.store(bodies.pending_bytes() as u64, Ordering::Release);

    // All senders dropped -- checkpoint WAL before closing connection.
    let span = tracing::debug_span!(
        target: "capsem.db",
        DB_SHUTDOWN_FLUSH_SPAN,
        status = tracing::field::Empty,
    );
    let started = Instant::now();
    let result = span.in_scope(|| conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)"));
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    let status = if result.is_ok() { "ok" } else { "error" };
    ::metrics::histogram!(DB_SHUTDOWN_FLUSH_MS, "status" => status).record(elapsed_ms);
    span.record("status", status);
}

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

fn flush_dirty_tables_to_disk(
    conn: &Connection,
    dirty_tables: &mut BTreeSet<&'static str>,
    flush_watermarks: &mut schema::MemoryFlushWatermarks,
    db_path: Option<&Path>,
    bodies: &mut BodyArchive,
    counters: &LedgerCounters,
) -> rusqlite::Result<()> {
    if dirty_tables.is_empty() && !bodies.has_work() {
        return Ok(());
    }
    // Bytes before index: the segment reaches the archive file here, and the
    // rows that name it are inserted in the transaction below. A crash in
    // between costs unreferenced bytes, never a row pointing past EOF.
    bodies.flush_segment();
    let tables: Vec<&'static str> = dirty_tables.iter().copied().collect();
    let tx = conn.unchecked_transaction()?;
    bodies.commit_index_rows(&tx)?;
    // Injected where a real flush failure lands: inside the transaction, with
    // the archive's index rows already written into it. Everything here rolls
    // back together, and the retry has to bring the bodies back with the rows.
    if take_disk_flush_failure_for_tests(db_path) {
        return Err(rusqlite::Error::InvalidParameterName(
            "injected disk flush failure before copy".to_string(),
        ));
    }
    let advanced_watermarks = schema::with_memory_schema_lock(|| {
        schema::flush_memory_tables_to_disk(&tx, tables.iter().copied(), flush_watermarks)
    })?;
    // The snapshot lands with the rows it counts, or neither does.
    counters::store(&tx, counters)?;
    tx.commit()?;
    // Only now are the written segments indexed: anything above this line rolls
    // the transaction back, and the next flush retries both halves together.
    bodies.index_rows_committed();
    flush_watermarks.extend(advanced_watermarks);
    if let Some(path) = db_path {
        schema::record_sqlite_mmap_telemetry(conn, path, "writer", "flush");
    }
    dirty_tables.clear();
    Ok(())
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
    insert_audit_event, insert_dns_event, insert_profile_mutation_event, insert_security_ask_event,
    insert_security_decision_event, insert_security_rule_event, insert_substitution_event,
};
use traffic_rows::{insert_exec_event, insert_file_event, insert_mcp_call, insert_net_event, update_exec_event};

#[cfg(test)]
mod tests;
