//! The writer thread's half of the body archive.
//!
//! Bodies are fed to the archive's open block as their event is written, and
//! the index rows that name them are held until the bytes they point at are
//! on disk. The order is the whole point: every disk flush writes one segment
//! of the open block -- everything staged since the last one -- and syncs it
//! *before* the transaction that inserts its rows and updates its
//! `body_blocks` row commits. Periodic flushes perform that device sync on a
//! small dedicated thread so the ledger can keep accepting events, while
//! explicit barriers wait for it. A crash between the two leaves unreferenced
//! bytes at the end of the file, which cost their bytes and nothing else; the
//! reverse order would leave index rows pointing past EOF, which is a ledger
//! that lies.
//!
//! A sync-flushed segment leaves the block open, so later bodies reuse its
//! dictionary. A block closes at its target size, `MAX_BLOCK_AGE`, shutdown,
//! or retention. Compression runs synchronously on this writer thread; only
//! the durability syscall leaves it.
//!
//! Identical bytes within the open block are stored once. An event that
//! matches three rules archives its payload three times over, and the
//! decision it drives and an ask it raises repeat it again; a body whose
//! stored bytes hash to one already in the open block is indexed against that
//! span and adds nothing to the file. A body of at least 512 bytes whose
//! bytes already sit in a committed segment of any earlier block is indexed
//! against that span too. Retention keeps a block while any row it retains
//! names it, so a shared span lives as long as its newest reader.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use super::{blake3_bytes_ref, execute_cached, format_timestamp, LedgerClock, MAX_BODY_BLOB_BYTES};
use capsem_archive::{
    ArchiveCodecs, ArchiveError, ArchiveId, BodyLogWriter, BodyRef, FileHeader, GenerationId, SegmentWritten,
};
use capsem_foundation::unix::contained::{ContainedDir, EntryKind};
use capsem_foundation::unix::fs::{durable_sync_directory, ensure_private_dir};
use capsem_foundation::unix::lock::{self, FileLock, LockAttempt, LockMode};
use capsem_telemetry::db::{DB_ARCHIVE_BODIES_DEDUPLICATED_TOTAL, DB_ARCHIVE_BODIES_DROPPED_TOTAL};
use rusqlite::{params, Connection, OptionalExtension};
use tracing::warn;

mod index;
mod sync_worker;
use sync_worker::ArchiveSyncWorker;

/// How long a block may stay open before the next body starts a new one.
///
/// Retention drops whole blocks and keeps a block while its newest segment
/// is inside the retention window, so a block that stayed open for a day
/// would keep a day of bodies past their cutoff. Retention is counted in
/// days, and an hour keeps it precise to within one. Shorter costs ratio on a
/// sparse session: replaying a ten-day ledger with a burst every few hours,
/// fifteen minutes gave 9.7x, an hour 10.4x, and never closing on age 11.2x.
pub(super) const MAX_BLOCK_AGE: Duration = Duration::from_secs(60 * 60);

/// Archives whose every flush closes the block, as every flush sealed one
/// before blocks stayed open. For the tests whose subject is what happens to
/// separate blocks -- retention by age, the dedup boundary -- rather than
/// when a block closes.
#[cfg(test)]
static CLOSE_AT_EVERY_FLUSH: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

/// Make every flush of the ledger at `db_path` close its archive block.
#[cfg(test)]
pub(crate) fn close_blocks_at_every_flush_for_tests(db_path: &Path) {
    CLOSE_AT_EVERY_FLUSH
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(archive_path_for_db(db_path));
}

/// One body to archive, as its `insert_*` writer describes it.
pub(super) struct EventBodyBlob<'a> {
    pub(super) event_id: &'a str,
    pub(super) event_type: &'static str,
    pub(super) source_table: &'static str,
    pub(super) direction: &'static str,
    pub(super) content_type: Option<&'a str>,
    pub(super) body: Option<&'a [u8]>,
    /// What the producer says the whole body was, when `body` is already a
    /// capped excerpt of it. `None` means `body` is the whole thing.
    pub(super) original_bytes: Option<u64>,
    pub(super) trace_id: Option<&'a str>,
    pub(super) turn_id: Option<&'a str>,
}

/// An index row waiting for the segment holding its bytes to reach the file.
struct BodyIndexRow {
    /// Monotonic across the archive, so a rolled-back transaction can drop
    /// its own rows wherever they ended up -- including behind a segment
    /// that was flushed while the transaction was still open.
    seq: u64,
    event_id: String,
    event_type: &'static str,
    source_table: &'static str,
    direction: &'static str,
    content_type: Option<String>,
    original_bytes: i64,
    stored_bytes: i64,
    truncated: bool,
    body_hash: String,
    block_offset: i64,
    body_offset: i64,
    trace_id: Option<String>,
    turn_id: Option<String>,
    created_at: String,
}

struct AppendedSegment {
    segment: SegmentWritten,
    rows: Vec<BodyIndexRow>,
    block_committed: bool,
}

/// The writer thread's archive: one `session.bodies` beside `session.db`.
pub(super) struct BodyArchive {
    db_path: Option<PathBuf>,
    /// The private generation directory, or `None` for an in-memory database.
    directory: Option<ContainedDir>,
    /// `None` for an in-memory database, which has no file to put an archive
    /// beside, and after a write failure, which makes every later offset
    /// unprovable. Both stage nothing rather than writing an index nobody can
    /// resolve.
    writer: Option<BodyLogWriter>,
    codecs: ArchiveCodecs,
    /// What `created_at`, `sealed_at` and a block's age are read from. Every
    /// other column in the index is content, so this is the only value in it
    /// that a replay of the same session cannot reproduce -- and the only
    /// reason the fixture regenerator needs to supply its own.
    now: LedgerClock,
    /// When the open block took its first body; `None` with no block open.
    block_opened_at: Option<SystemTime>,
    /// Rows whose bytes are staged but not yet in a written segment.
    staged: Vec<BodyIndexRow>,
    /// Rows pointing at bytes an earlier transaction already committed. No
    /// segment carries them; they commit with the next flush.
    reused: Vec<BodyIndexRow>,
    /// Where each distinct body in the open block sits, by the hash of its
    /// stored bytes -- the same `blake3:` string the index row records, so it
    /// is computed once. Cleared when the block closes: a span is reusable
    /// while it is in the block being written.
    block_spans: HashMap<String, BodyRef>,
    /// Sequence number the next staged row takes.
    next_seq: u64,
    /// Segments written through the page cache and waiting to start a device
    /// sync.
    appended: Vec<AppendedSegment>,
    /// Segments whose device sync completed and whose rows may enter SQLite.
    synced: Vec<AppendedSegment>,
    /// Rows written by the current SQLite transaction. They leave `synced`
    /// and `reused` only after that transaction commits; a rollback retries
    /// the same prefix on the next flush.
    prepared_index_rows: Option<PreparedIndexRows>,
    /// Segments covered by the durability barrier currently running.
    syncing: Option<Vec<AppendedSegment>>,
    /// Lazily-created helper that keeps fdatasync off the admission thread.
    sync_worker: Option<ArchiveSyncWorker>,
    /// Test builds only: make the next segment write fail the way a full
    /// disk does. Injected at this seam rather than inside the archive
    /// writer because what is under test is what *this* type does once the
    /// writer is gone.
    #[cfg(test)]
    fail_next_append: bool,
    /// "sync" and "commit" in the order they happened, so a test can prove
    /// the archive is flushed before the rows that name it are written.
    #[cfg(test)]
    steps: Vec<&'static str>,
}

#[derive(Default)]
struct PreparedIndexRows {
    synced: usize,
    reused: usize,
    blocks: Vec<usize>,
}

impl PreparedIndexRows {
    const fn total(&self) -> usize {
        self.synced + self.reused
    }
}

/// Bodies shorter than this are not looked up in the archive index. Deflate's
/// window already finds a repeat that small inside the block, and the lookup
/// would cost more than the bytes it saves.
const MIN_ARCHIVE_DEDUP_BYTES: usize = 512;

/// Fifteen bound values per body row fit 64 rows below SQLite's portable
/// 999-variable ceiling. The statement and transient reference vector stay
/// bounded while a busy flush pays statement dispatch once per 64 bodies
/// instead of once per body.
/// Where identical stored bytes already sit in a committed segment, if they
/// do. Committed spans are immutable and synced, so pointing a new row at one
/// is as safe as pointing at fresh bytes; retention keeps a block while any
/// row it retains names it. A failed lookup is a missed saving, not an error.
fn committed_span(conn: &Connection, len: usize, body_hash: &str) -> Option<BodyRef> {
    if len < MIN_ARCHIVE_DEDUP_BYTES {
        return None;
    }
    let found = conn
        .prepare_cached(
            "SELECT block_offset, body_offset, body_len FROM main.event_body_blobs
             WHERE body_hash = ?1 LIMIT 1",
        )
        .and_then(|mut statement| {
            statement
                .query_row(params![body_hash], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?))
                })
                .optional()
        });
    match found {
        Ok(Some((block_offset, offset, body_len))) if usize::try_from(body_len).ok() == Some(len) => Some(BodyRef {
            block_offset: u64::try_from(block_offset).ok()?,
            offset: u32::try_from(offset).ok()?,
            len: u32::try_from(body_len).ok()?,
        }),
        Ok(_) => None,
        Err(error) => {
            warn!(error = %error, "archive dedup lookup failed; storing the body again");
            None
        }
    }
}

/// Whether a staging error is about this body or about the writer.
///
/// `Poisoned` is the writer saying it can no longer place bytes at all, so
/// everything staged against it is unplaceable too. `BodyTooLarge` is a
/// refusal of one body -- unreachable at the logger's 10 MiB cap, and if the
/// cap ever moves it must skip that body rather than end the session's
/// archive. Everything else is treated as a writer that can still be used.
pub(super) fn takes_the_archive_out_of_service(error: &ArchiveError) -> bool {
    matches!(error, ArchiveError::Poisoned)
}

/// Where a session's archive lives: `session.bodies` beside `session.db`.
pub(crate) fn archive_path_for_db(db_path: &Path) -> PathBuf {
    db_path.with_extension("bodies")
}

pub(crate) fn archive_lock_path_for_db(db_path: &Path) -> PathBuf {
    let mut name = OsString::from(db_path.as_os_str());
    name.push("-archive.lock");
    PathBuf::from(name)
}

fn archive_contract_error(message: impl Into<String>) -> rusqlite::Error {
    rusqlite::Error::InvalidParameterName(message.into())
}

impl BodyArchive {
    /// Prepare a fresh generation and stable acquisition lock before the
    /// schema transaction publishes their identities.
    pub(super) fn prepare_new(
        db_path: &Path,
        now: LedgerClock,
        codecs: ArchiveCodecs,
    ) -> rusqlite::Result<(Self, FileLock, FileHeader)> {
        let path = archive_path_for_db(db_path);
        match std::fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Ok(metadata) if metadata.is_file() => {
                return Err(archive_contract_error(format!(
                    "legacy v2 archive {} is a regular file; refusing to replace it",
                    path.display()
                )))
            }
            Ok(_) => {
                return Err(archive_contract_error(format!(
                    "unpublished archive path {} already exists; refusing to guess or erase it",
                    path.display()
                )))
            }
            Err(error) => return Err(archive_contract_error(format!("inspect {}: {error}", path.display()))),
        }
        ensure_private_dir(&path)
            .map_err(|error| archive_contract_error(format!("create {}: {error}", path.display())))?;
        let parent = path
            .parent()
            .ok_or_else(|| archive_contract_error("archive directory has no parent"))?;
        durable_sync_directory(parent)
            .map_err(|error| archive_contract_error(format!("sync {}: {error}", parent.display())))?;

        let lock_path = archive_lock_path_for_db(db_path);
        let archive_lock = match lock::try_acquire(&lock_path, LockMode::Exclusive)
            .map_err(|error| archive_contract_error(format!("create {}: {error}", lock_path.display())))?
        {
            LockAttempt::Acquired(lock) => lock,
            LockAttempt::Contended => {
                return Err(archive_contract_error(format!(
                    "fresh archive lock {} is unexpectedly contended",
                    lock_path.display()
                )))
            }
        };
        durable_sync_directory(parent)
            .map_err(|error| archive_contract_error(format!("sync {}: {error}", parent.display())))?;

        let directory = ContainedDir::open_root(&path)
            .and_then(|directory| {
                directory.validate_private()?;
                Ok(directory)
            })
            .map_err(|error| archive_contract_error(format!("open {}: {error}", path.display())))?;
        let header = FileHeader {
            archive_id: ArchiveId::generate(),
            generation_id: GenerationId::generate(),
        };
        let mut writer = BodyLogWriter::create_generation_with_codecs(
            &directory,
            header.archive_id,
            header.generation_id,
            codecs.clone(),
        )
        .map_err(|error| archive_contract_error(format!("create generation: {error}")))?;
        writer
            .sync()
            .map_err(|error| archive_contract_error(format!("sync generation: {error}")))?;
        directory
            .sync()
            .map_err(|error| archive_contract_error(format!("sync {}: {error}", path.display())))?;
        Ok((
            Self::with_writer(db_path.to_path_buf(), directory, writer, now, codecs),
            archive_lock,
            header,
        ))
    }

    /// Recover and reopen the exact generation selected by SQLite. The
    /// archive lock stays exclusive through the real revision durability
    /// fence; no path scan can elect another file.
    pub(super) fn open_existing(
        db_path: &Path,
        now: LedgerClock,
        conn: &Connection,
        codecs: ArchiveCodecs,
    ) -> rusqlite::Result<Self> {
        let path = archive_path_for_db(db_path);
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|error| archive_contract_error(format!("inspect {}: {error}", path.display())))?;
        if metadata.is_file() {
            return Err(archive_contract_error(format!(
                "legacy v2 archive {} is a regular file; refusing format v3 open",
                path.display()
            )));
        }
        let directory = ContainedDir::open_root(&path)
            .and_then(|directory| {
                directory.validate_private()?;
                Ok(directory)
            })
            .map_err(|error| archive_contract_error(format!("open {}: {error}", path.display())))?;
        let lock_path = archive_lock_path_for_db(db_path);
        let _archive_lock =
            lock::acquire_existing_until(&lock_path, LockMode::Exclusive, Instant::now() + Duration::from_secs(5))
                .map_err(|error| archive_contract_error(format!("lock {}: {error}", lock_path.display())))?;
        let state = crate::schema::archive_state(conn)?;
        let writer =
            BodyLogWriter::open_generation_with_codecs(&directory, state.header, state.committed_end, codecs.clone())
                .map_err(|error| archive_contract_error(format!("open active generation: {error}")))?;
        let next_revision = state
            .revision
            .checked_add(1)
            .and_then(|revision| i64::try_from(revision).ok())
            .ok_or_else(|| archive_contract_error("archive_state revision overflow"))?;
        let updated = conn.execute(
            "UPDATE archive_state SET revision = ?1 WHERE singleton = 1 AND revision = ?2",
            rusqlite::params![next_revision, state.revision as i64],
        )?;
        if updated != 1 {
            return Err(archive_contract_error("archive_state changed during writer recovery"));
        }
        Self::gc_unreferenced_generations(&directory, state.header.generation_id);
        Ok(Self::with_writer(db_path.to_path_buf(), directory, writer, now, codecs))
    }

    /// Retry deletion of exact managed generation names after the elected
    /// generation has been validated and fenced. One candidate is selected
    /// per directory walk so cleanup remains bounded even if a crash left a
    /// very large number of candidates.
    fn gc_unreferenced_generations(directory: &ContainedDir, active: GenerationId) {
        loop {
            let mut candidate = None;
            let walk = directory.visit_entries(|entry| {
                let Some(name) = entry.name.to_str() else {
                    warn!(name = ?entry.name, "archive directory contains a non-UTF-8 entry; retaining it");
                    return Ok(true);
                };
                let Ok(generation) = GenerationId::from_file_name(name) else {
                    warn!(name, "archive directory contains an unknown entry; retaining it");
                    return Ok(true);
                };
                if generation == active {
                    return Ok(true);
                }
                if entry.kind != EntryKind::File {
                    warn!(name, ?entry.kind, "archive candidate is not a regular file; retaining it");
                    return Ok(true);
                }
                candidate = Some(entry.name);
                Ok(false)
            });
            if let Err(error) = walk {
                warn!(error = %error, "archive generation GC could not enumerate candidates");
                return;
            }
            let Some(name) = candidate else {
                return;
            };
            if let Err(error) = directory.remove_private_file(&name).and_then(|()| directory.sync()) {
                warn!(name = ?name, error = %error, "archive generation GC remains pending");
                return;
            }
        }
    }

    pub(super) fn disabled(now: LedgerClock) -> Self {
        Self::new(None, None, None, now, ArchiveCodecs::default())
    }

    #[cfg(test)]
    pub(super) fn open_for_tests(db_path: Option<&Path>, now: LedgerClock, conn: &Connection) -> Self {
        let Some(db_path) = db_path else {
            return Self::disabled(now);
        };
        let path = archive_path_for_db(db_path);
        let opened = if path.exists() {
            Self::open_existing(db_path, now, conn, ArchiveCodecs::default())
        } else {
            let state = crate::schema::archive_state(conn);
            state.and_then(|state| {
                ensure_private_dir(&path).map_err(|error| archive_contract_error(error.to_string()))?;
                let parent = path
                    .parent()
                    .ok_or_else(|| archive_contract_error("archive directory has no parent"))?;
                durable_sync_directory(parent).map_err(|error| archive_contract_error(error.to_string()))?;
                let archive_lock = lock::try_acquire(&archive_lock_path_for_db(db_path), LockMode::Exclusive)
                    .map_err(|error| archive_contract_error(error.to_string()))?;
                drop(archive_lock);
                let directory =
                    ContainedDir::open_root(&path).map_err(|error| archive_contract_error(error.to_string()))?;
                let mut writer =
                    BodyLogWriter::create_generation(&directory, state.header.archive_id, state.header.generation_id)
                        .map_err(|error| archive_contract_error(error.to_string()))?;
                writer
                    .sync()
                    .map_err(|error| archive_contract_error(error.to_string()))?;
                directory
                    .sync()
                    .map_err(|error| archive_contract_error(error.to_string()))?;
                Ok(Self::with_writer(
                    db_path.to_path_buf(),
                    directory,
                    writer,
                    now,
                    ArchiveCodecs::default(),
                ))
            })
        };
        opened.unwrap_or_else(|error| {
            warn!(error = %error, "test archive could not be opened");
            Self::new(Some(db_path.to_path_buf()), None, None, now, ArchiveCodecs::default())
        })
    }

    fn with_writer(
        db_path: PathBuf,
        directory: ContainedDir,
        writer: BodyLogWriter,
        now: LedgerClock,
        codecs: ArchiveCodecs,
    ) -> Self {
        Self::new(Some(db_path), Some(directory), Some(writer), now, codecs)
    }

    fn new(
        db_path: Option<PathBuf>,
        directory: Option<ContainedDir>,
        writer: Option<BodyLogWriter>,
        now: LedgerClock,
        codecs: ArchiveCodecs,
    ) -> Self {
        Self {
            db_path,
            directory,
            writer,
            codecs,
            now,
            block_opened_at: None,
            staged: Vec::new(),
            reused: Vec::new(),
            block_spans: HashMap::new(),
            next_seq: 0,
            appended: Vec::new(),
            synced: Vec::new(),
            prepared_index_rows: None,
            syncing: None,
            sync_worker: None,
            #[cfg(test)]
            fail_next_append: false,
            #[cfg(test)]
            steps: Vec::new(),
        }
    }

    pub(super) fn codecs(&self) -> ArchiveCodecs {
        self.codecs.clone()
    }

    /// Stage one body: its bytes into the open block, its index row beside
    /// them. Empty and absent bodies produce neither.
    pub(super) fn stage(&mut self, conn: &Connection, blob: EventBodyBlob<'_>) {
        if self.writer.is_none() {
            return;
        }
        let Some(bytes) = blob.body.filter(|body| !body.is_empty()) else {
            return;
        };
        let stored_len = bytes.len().min(MAX_BODY_BLOB_BYTES);
        // What the producer sent may already be an excerpt -- guest exec
        // output is capped at the vsock boundary -- and then the row must
        // report the size it was cut from, not the size that arrived. A
        // producer that reports less than it sent is not believed: the floor
        // is what actually arrived, so under-reporting cannot also erase the
        // truncation this writer did on top of it.
        let original_bytes = blob.original_bytes.unwrap_or(0).max(bytes.len() as u64);
        let stored = &bytes[..stored_len];
        // Over the stored bytes, not the buffer they were cut from: a hash of
        // something the archive does not hold cannot be checked against
        // anything, and a reader that verifies what it read is how a corrupted
        // index row stops being a body. It is also what finds a repeat.
        let body_hash = blake3_bytes_ref(stored);
        let Some((reference, committed)) = self.stage_bytes(conn, stored, &body_hash) else {
            return;
        };
        let seq = self.next_seq;
        self.next_seq += 1;
        // A row naming bytes already committed and synced waits for no
        // segment: it joins the next transaction as it is.
        let pending = if committed { &mut self.reused } else { &mut self.staged };
        pending.push(BodyIndexRow {
            seq,
            event_id: blob.event_id.to_string(),
            event_type: blob.event_type,
            source_table: blob.source_table,
            direction: blob.direction,
            content_type: blob.content_type.map(str::to_string),
            original_bytes: original_bytes as i64,
            stored_bytes: stored_len as i64,
            // Per row, even when the span is shared: two producers can cut the
            // same stored bytes from bodies of different sizes.
            truncated: original_bytes > stored_len as u64,
            body_hash,
            block_offset: reference.block_offset as i64,
            body_offset: i64::from(reference.offset),
            trace_id: blob.trace_id.map(str::to_string),
            turn_id: blob.turn_id.map(str::to_string),
            created_at: format_timestamp((self.now)()),
        });
    }

    /// Stage bytes into the open block -- or, when the open block already
    /// holds these exact bytes, hand back their span and add nothing.
    ///
    /// Keyed by the blake3 of the bytes, never by anything weaker: a length or
    /// a prefix match would index one body against another's bytes, and the
    /// reader's hash check would then refuse it forever.
    ///
    /// A block open longer than `MAX_BLOCK_AGE` is closed first, so this body
    /// starts a new one. `BlockFull` is the archive's close-and-retry
    /// contract, not a failure: the open block is within 16 MiB of its ceiling
    /// and this body does not fit beside what is already there. Closing
    /// empties it, and a body capped at `MAX_BODY_BLOB_BYTES` (10 MiB) always
    /// fits an empty one, so the retry is the last step and not a loop.
    ///
    /// Every other error skips the body and logs. A poisoned writer takes the
    /// archive out of service here, because the rows staged beside it would
    /// name bytes no file received. In no case may a body take down the
    /// writer thread that owns the whole session ledger.
    fn stage_bytes(&mut self, conn: &Connection, bytes: &[u8], body_hash: &str) -> Option<(BodyRef, bool)> {
        if self.block_is_stale() {
            self.close_block();
        }
        if let Some(reference) = self.block_spans.get(body_hash) {
            ::metrics::counter!(DB_ARCHIVE_BODIES_DEDUPLICATED_TOTAL, "scope" => "block").increment(1);
            return Some((*reference, false));
        }
        if let Some(reference) = committed_span(conn, bytes.len(), body_hash) {
            ::metrics::counter!(DB_ARCHIVE_BODIES_DEDUPLICATED_TOTAL, "scope" => "archive").increment(1);
            return Some((reference, true));
        }
        match self.stage_into_open_block(bytes, body_hash)? {
            Ok(reference) => return Some((reference, false)),
            Err(ArchiveError::BlockFull) => {}
            Err(error) => {
                self.refuse_body(&error, bytes.len(), "stage");
                return None;
            }
        }
        self.close_block();
        match self.stage_into_open_block(bytes, body_hash) {
            Some(Ok(reference)) => Some((reference, false)),
            Some(Err(error)) => {
                self.refuse_body(&error, bytes.len(), "stage_after_close");
                None
            }
            None => {
                // `close_block` failed its write and retired the writer. The
                // count it took covered the rows it was holding; this body has
                // no row yet, so it was not among them. Without this line it is
                // the one body the counter never sees.
                warn!(body_bytes = bytes.len(), "body not archived after close");
                self.drop_bodies(1, "stage_after_close");
                None
            }
        }
    }

    /// One attempt at staging: `None` when there is no writer.
    fn stage_into_open_block(&mut self, bytes: &[u8], body_hash: &str) -> Option<Result<BodyRef, ArchiveError>> {
        let writer = self.writer.as_mut()?;
        let opening = writer.open_block_raw_len().is_none();
        let staged = writer.stage(bytes);
        if let Ok(reference) = &staged {
            if opening {
                self.block_opened_at = Some((self.now)());
            }
            self.block_spans.insert(body_hash.to_string(), *reference);
        }
        Some(staged)
    }

    fn refuse_body(&mut self, error: &ArchiveError, body_bytes: usize, reason: &'static str) {
        warn!(error = %error, body_bytes, "body not archived");
        if takes_the_archive_out_of_service(error) {
            self.give_up(reason);
        }
        self.drop_bodies(1, reason);
    }

    /// Whether the open block has been open longer than `MAX_BLOCK_AGE`. A
    /// clock that went backwards reads as not stale.
    fn block_is_stale(&self) -> bool {
        self.block_opened_at
            .and_then(|opened| (self.now)().duration_since(opened).ok())
            .is_some_and(|age| age >= MAX_BLOCK_AGE)
    }

    /// Take the archive out of service, dropping the rows it can no longer
    /// place.
    ///
    /// A poisoned writer refuses every later call, so the rows staged against
    /// its open block will never have bytes to point at. Keeping them would
    /// leave the next flush inserting index rows for bytes no file received,
    /// which is the one outcome the whole split exists to prevent.
    pub(super) fn give_up(&mut self, reason: &'static str) {
        // Close first so a writer that could still place bytes leaves the
        // file ending on a whole block. A poisoned one refuses, which is the
        // case this exists for; anything else leaves the bytes merely
        // unreferenced.
        self.close_block();
        self.abandon_uncommitted(reason);
        self.retire_writer();
    }

    /// Stop archiving: the writer goes, and whatever it had staged but not
    /// written goes with it. Every caller has already counted the rows that
    /// named those bytes.
    fn retire_writer(&mut self) {
        self.sync_worker = None;
        if let Some(writer) = self.writer.take() {
            writer.abandon();
        }
    }

    /// The archive file this session writes, when it has one and the writer
    /// is still in service. `None` is retention's cue that there is nothing
    /// it may rewrite: an in-memory ledger has no file, and an archive that
    /// gave up must not have its file compacted underneath index rows it can
    /// no longer vouch for.
    pub(super) fn generation_directory(&self) -> Option<&ContainedDir> {
        self.writer.as_ref().and(self.directory.as_ref())
    }

    pub(super) fn db_path(&self) -> Option<&Path> {
        self.writer.as_ref().and(self.db_path.as_deref())
    }

    pub(super) fn adopt_generation(&mut self, writer: BodyLogWriter) {
        debug_assert!(!self.has_work());
        self.sync_worker = None;
        self.writer = Some(writer);
        self.block_spans.clear();
        self.block_opened_at = None;
    }

    /// The sequence number the next staged row will take: a transaction's
    /// mark, taken before it stages anything.
    pub(super) fn staged_mark(&self) -> u64 {
        self.next_seq
    }

    /// Drop the index rows staged since `mark`, after the transaction that
    /// would have owned them rolled back. Their bytes stay where they are --
    /// unreferenced, and cheaper than rewriting every later offset. Rows
    /// behind a segment that was written mid-transaction go with the rest.
    pub(super) fn rollback_staged(&mut self, mark: u64) {
        self.staged.retain(|row| row.seq < mark);
        self.reused.retain(|row| row.seq < mark);
        for segment in &mut self.appended {
            segment.rows.retain(|row| row.seq < mark);
        }
        for segment in &mut self.synced {
            segment.rows.retain(|row| row.seq < mark);
        }
        if let Some(syncing) = &mut self.syncing {
            for segment in syncing {
                segment.rows.retain(|row| row.seq < mark);
            }
        }
    }

    /// Close the open block if it is full or too old. Called between the
    /// operations of a batch as well as after it, so one large batch cannot
    /// grow a block far past its target.
    pub(super) fn close_if_due(&mut self) {
        if self.writer.as_ref().is_some_and(BodyLogWriter::wants_close) || self.block_is_stale() {
            self.close_block();
        }
    }

    /// Raw bytes staged but not yet in a written segment.
    pub(super) fn pending_bytes(&self) -> usize {
        self.writer.as_ref().map_or(0, BodyLogWriter::pending_bytes)
    }

    /// Whether a flush has anything to do here: rows waiting to commit, or
    /// bytes waiting for a segment. Pending bytes count even when their rows
    /// were dropped by a rollback -- they still have to reach the file, or the
    /// writer closes holding them.
    pub(super) fn has_work(&self) -> bool {
        !self.appended.is_empty()
            || !self.synced.is_empty()
            || self.syncing.is_some()
            || !self.staged.is_empty()
            || !self.reused.is_empty()
            || self.pending_bytes() > 0
    }

    /// Write everything staged since the last flush as one segment of the
    /// open block -- or close the block, when it is due. Its index rows move
    /// with it and wait for the transaction.
    pub(super) fn flush_segment(&mut self) {
        if self.writer.as_ref().is_some_and(BodyLogWriter::wants_close)
            || self.block_is_stale()
            || self.closes_at_every_flush()
        {
            self.close_block();
        } else {
            self.write_segment(false);
        }
    }

    /// End the open block with a FINAL segment carrying anything still
    /// staged. The next body opens a new block.
    pub(super) fn close_block(&mut self) {
        self.write_segment(true);
        self.block_spans.clear();
        self.block_opened_at = None;
    }

    fn write_segment(&mut self, close: bool) {
        // Taken before the writer is borrowed, and only in test builds.
        #[cfg(test)]
        let injected_failure = std::mem::take(&mut self.fail_next_append);
        let Some(writer) = self.writer.as_mut() else {
            return;
        };
        #[cfg(test)]
        let written = if injected_failure {
            Err(ArchiveError::Io(std::io::Error::other("injected append failure")))
        } else if close {
            writer.close_block()
        } else {
            writer.flush_segment()
        };
        #[cfg(not(test))]
        let written = if close {
            writer.close_block()
        } else {
            writer.flush_segment()
        };
        match written {
            Ok(Some(segment)) => {
                let rows = std::mem::take(&mut self.staged);
                self.appended.push(AppendedSegment {
                    segment,
                    rows,
                    block_committed: false,
                });
            }
            Ok(None) => {}
            Err(error) => {
                // The file's end is no longer provably where the writer
                // believes it is, so every later offset would be a guess.
                // Stop archiving rather than index bytes we cannot name.
                warn!(
                    error = %error,
                    dropped_bodies = self.staged.len(),
                    "session body archive write failed; no further bodies will be stored"
                );
                let rows = std::mem::take(&mut self.staged);
                self.drop_bodies(rows.len(), "append");
                self.retire_writer();
            }
        }
    }

    /// Start or collect the durability work that makes index rows safe.
    ///
    /// Periodic flushes poll without waiting so archive storage latency never
    /// blocks new ledger admissions. Explicit barriers and shutdown wait for
    /// every segment, preserving their read-after-write and durability
    /// contracts.
    pub(super) fn prepare_index_rows(&mut self, wait: bool) {
        if !self.finish_archive_sync(wait) {
            return;
        }
        while self.syncing.is_none() && !self.appended.is_empty() {
            if !self.start_archive_sync() || !wait || !self.finish_archive_sync(true) {
                return;
            }
        }
    }

    fn start_archive_sync(&mut self) -> bool {
        if self.appended.is_empty() {
            return true;
        }
        if self.writer.is_none() {
            self.abandon_uncommitted("poisoned");
            return false;
        }
        if self.sync_worker.is_none() {
            let started = self
                .writer
                .as_ref()
                .expect("writer checked above")
                .sync_handle()
                .map_err(|error| error.to_string())
                .and_then(|sync| ArchiveSyncWorker::start(sync).map_err(|error| error.to_string()));
            match started {
                Ok(worker) => self.sync_worker = Some(worker),
                Err(error) => {
                    warn!(error, "session body archive sync worker could not start");
                    self.abandon_uncommitted("sync");
                    self.retire_writer();
                    return false;
                }
            }
        }
        if let Err(error) = self.sync_worker.as_ref().expect("worker created above").request() {
            warn!(error, "session body archive sync worker stopped");
            self.abandon_uncommitted("sync");
            self.retire_writer();
            return false;
        }
        self.syncing = Some(std::mem::take(&mut self.appended));
        true
    }

    fn finish_archive_sync(&mut self, wait: bool) -> bool {
        if self.syncing.is_none() {
            return true;
        }
        let result = if wait {
            Some(
                self.sync_worker
                    .as_ref()
                    .expect("an in-flight sync has a worker")
                    .complete(),
            )
        } else {
            self.sync_worker
                .as_ref()
                .expect("an in-flight sync has a worker")
                .try_complete()
        };
        let Some(result) = result else {
            return true;
        };
        let syncing = self.syncing.take().expect("the completed sync was in flight");
        match result {
            Ok(()) => {
                self.synced.extend(syncing);
                #[cfg(test)]
                self.steps.push("sync");
                true
            }
            Err(error) => {
                let dropped_segments = syncing.len();
                self.appended.extend(syncing);
                warn!(
                    error,
                    dropped_segments, "session body archive could not be flushed; no further bodies will be stored"
                );
                self.abandon_uncommitted("sync");
                self.retire_writer();
                false
            }
        }
    }

    /// Durability barrier for session close. The block was closed and its
    /// rows committed by the final flush; this makes sure the device has it.
    pub(super) fn sync(&mut self) {
        if let Some(writer) = self.writer.as_mut() {
            if let Err(error) = writer.sync() {
                warn!(error = %error, "session body archive sync failed");
            }
        }
    }

    /// Give up on every body this archive is still holding a row for, counting
    /// them under `reason`. Both halves go: a segment waiting to be indexed
    /// and the rows of bytes not yet written are equally unreachable once the
    /// archive stops vouching for its own file.
    pub(super) fn abandon_uncommitted(&mut self, reason: &'static str) {
        let dropped: usize = self.appended.iter().map(|segment| segment.rows.len()).sum::<usize>()
            + self.synced.iter().map(|segment| segment.rows.len()).sum::<usize>()
            + self
                .syncing
                .as_ref()
                .map_or(0, |segments| segments.iter().map(|segment| segment.rows.len()).sum())
            + self.staged.len()
            + self.reused.len();
        self.appended.clear();
        self.synced.clear();
        self.syncing = None;
        self.staged.clear();
        self.reused.clear();
        self.drop_bodies(dropped, reason);
    }

    /// Count bodies the archive gave up on. A session whose archive poisoned
    /// itself keeps serving every other ledger row, so nothing downstream
    /// looks wrong; this counter is how that shows up anywhere but a log line.
    fn drop_bodies(&self, count: usize, reason: &'static str) {
        if count == 0 {
            return;
        }
        ::metrics::counter!(DB_ARCHIVE_BODIES_DROPPED_TOTAL, "reason" => reason).increment(count as u64);
    }

    /// Whether a test asked this archive to close its block at every flush.
    fn closes_at_every_flush(&self) -> bool {
        #[cfg(test)]
        if let Some(path) = self.db_path.as_deref().map(archive_path_for_db) {
            return CLOSE_AT_EVERY_FLUSH
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .iter()
                .any(|configured| configured == &path);
        }
        false
    }

    /// Put this archive in the state a failed write leaves: no writer, and
    /// whatever segments had already been written still waiting for rows.
    #[cfg(test)]
    pub(super) fn poison_writer_for_tests(&mut self) {
        self.retire_writer();
    }

    /// Make the next segment write fail, as a full disk would.
    #[cfg(test)]
    pub(super) fn fail_next_append_for_tests(&mut self) {
        self.fail_next_append = true;
    }

    /// Segments written but not yet indexed.
    #[cfg(test)]
    pub(super) fn appended_len_for_tests(&self) -> usize {
        self.appended.len() + self.synced.len() + self.syncing.as_ref().map_or(0, Vec::len)
    }

    /// The flush/commit steps this archive has taken, in order.
    #[cfg(test)]
    pub(super) fn steps_for_tests(&self) -> &[&'static str] {
        &self.steps
    }
}
