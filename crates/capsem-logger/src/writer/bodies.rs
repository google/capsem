//! The writer thread's half of the body archive.
//!
//! Bodies are fed to the archive's open block as their event is written, and
//! the index rows that name them are held until the bytes they point at are
//! on disk. The order is the whole point: every disk flush writes one segment
//! of the open block -- everything staged since the last one -- and syncs it
//! *before* the transaction that inserts its rows and updates its
//! `body_blocks` row commits. A crash between the two leaves unreferenced
//! bytes at the end of the file, which cost their bytes and nothing else; the
//! reverse order would leave index rows pointing past EOF, which is a ledger
//! that lies.
//!
//! The block stays open across flushes: a segment is sync-flushed, not
//! sealed, so the next one compresses against the same dictionary. Before
//! this, every five-second flush sealed its block, real blocks averaged about
//! 85 KiB, and the flush timer rather than the data capped the compression
//! ratio. A block closes when it reaches the archive's target size, when it
//! has been open for `MAX_BLOCK_AGE` -- so retention, which drops blocks
//! whole, stays precise on a quiet session -- and at shutdown and retention.
//!
//! Compression runs on this thread, synchronously: each body is fed to the
//! compressor as it is staged, and a flush costs only the sync point.
//!
//! Identical bytes within the open block are stored once. An event that
//! matches three rules archives its payload three times over, and the
//! decision it drives and an ask it raises repeat it again; a body whose
//! stored bytes hash to one already in the open block is indexed against that
//! span and adds nothing to the file. The reuse stays inside one block:
//! retention drops blocks whole, so every row naming a block shares that
//! block's lifetime.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use capsem_archive::{ArchiveError, BodyLogWriter, BodyRef, SegmentWritten};
use rusqlite::{params, Connection};
use tracing::warn;

use super::{
    blake3_bytes_ref, execute_cached, format_timestamp, LedgerClock, DB_ARCHIVE_BODIES_DEDUPLICATED_TOTAL,
    MAX_BODY_BLOB_BYTES,
};

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
    block_offset: u64,
    body_offset: i64,
    trace_id: Option<String>,
    turn_id: Option<String>,
    created_at: String,
}

/// The writer thread's archive: one `session.bodies` beside `session.db`.
pub(super) struct BodyArchive {
    /// Where the archive file is, or `None` for an in-memory database. Kept
    /// even when the writer is gone, because retention rewrites the file by
    /// path and then reopens the writer on it.
    path: Option<PathBuf>,
    /// `None` for an in-memory database, which has no file to put an archive
    /// beside, and after a write failure, which makes every later offset
    /// unprovable. Both stage nothing rather than writing an index nobody can
    /// resolve.
    writer: Option<BodyLogWriter>,
    /// What `created_at`, `sealed_at` and a block's age are read from. Every
    /// other column in the index is content, so this is the only value in it
    /// that a replay of the same session cannot reproduce -- and the only
    /// reason the fixture regenerator needs to supply its own.
    now: LedgerClock,
    /// When the open block took its first body; `None` with no block open.
    block_opened_at: Option<SystemTime>,
    /// Rows whose bytes are staged but not yet in a written segment.
    staged: Vec<BodyIndexRow>,
    /// Where each distinct body in the open block sits, by the hash of its
    /// stored bytes -- the same `blake3:` string the index row records, so it
    /// is computed once. Cleared when the block closes: a span is reusable
    /// while it is in the block being written.
    block_spans: HashMap<String, BodyRef>,
    /// Sequence number the next staged row takes.
    next_seq: u64,
    /// Segments on disk, each with the rows it made placeable, waiting for
    /// those rows to commit.
    appended: Vec<(SegmentWritten, Vec<BodyIndexRow>)>,
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

/// Whether every block extent the index names is inside the archive file.
///
/// Retention replaces the archive and rewrites the index, and the two cannot
/// commit together -- there is one `rename(2)` between them. A crash in that
/// width leaves the index naming offsets the file on disk does not have, and
/// until this check existed nothing noticed: the writer reopened whichever
/// file was there and appended into it, and every stale row stayed in the
/// index, answering reads with another block's bytes until its hash check
/// refused them, one body at a time, forever.
///
/// So it is checked once, at open, where it can be said plainly and where
/// refusing costs only this session's new bodies. A `body_blocks` that cannot
/// be read is broken schema, not an empty table.
fn index_fits_the_file(conn: &Connection, archive_path: &Path) -> bool {
    let indexed_end: Option<i64> =
        match conn.query_row("SELECT MAX(block_offset + disk_len) FROM body_blocks", [], |row| {
            row.get(0)
        }) {
            Ok(end) => end,
            Err(error) => {
                warn!(
                    archive_path = %archive_path.display(),
                    error = %error,
                    "session body index could not be read; bodies will not be stored"
                );
                return false;
            }
        };
    let Some(indexed_end) = indexed_end else {
        // No blocks indexed: nothing to disagree with, including for a
        // session whose archive file does not exist yet.
        return true;
    };
    let archive_bytes = match std::fs::metadata(archive_path) {
        Ok(metadata) => metadata.len() as i64,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => {
            warn!(
                archive_path = %archive_path.display(),
                error = %error,
                "session body archive could not be measured; bodies will not be stored"
            );
            return false;
        }
    };
    if indexed_end > archive_bytes {
        warn!(
            archive_path = %archive_path.display(),
            indexed_end,
            archive_bytes,
            "session body index names bytes past the end of the archive, so the two no longer \
             describe the same file -- most likely a crash during retention; bodies will not be \
             stored and existing rows will fail their integrity check rather than answer wrongly"
        );
        return false;
    }
    true
}

/// Open the archive writer, or warn and store no bodies. Shared by `open` and
/// by the reopen retention needs: a compacted file has a new end, and a writer
/// still holding the old one would append over a kept block.
fn open_writer(archive_path: &Path) -> Option<BodyLogWriter> {
    match BodyLogWriter::open(archive_path) {
        Ok(writer) => Some(writer),
        Err(error) => {
            warn!(
                archive_path = %archive_path.display(),
                error = %error,
                "session body archive could not be opened; bodies will not be stored"
            );
            None
        }
    }
}

impl BodyArchive {
    /// Open the session's archive, after checking that the index and the
    /// file still describe the same thing.
    pub(super) fn open(db_path: Option<&Path>, now: LedgerClock, conn: &Connection) -> Self {
        let path = db_path.map(archive_path_for_db);
        let writer = path
            .as_deref()
            .filter(|path| index_fits_the_file(conn, path))
            .and_then(open_writer);
        Self {
            path,
            writer,
            now,
            block_opened_at: None,
            staged: Vec::new(),
            block_spans: HashMap::new(),
            next_seq: 0,
            appended: Vec::new(),
            #[cfg(test)]
            fail_next_append: false,
            #[cfg(test)]
            steps: Vec::new(),
        }
    }

    /// Stage one body: its bytes into the open block, its index row beside
    /// them. Empty and absent bodies produce neither.
    pub(super) fn stage(&mut self, blob: EventBodyBlob<'_>) {
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
        let Some(reference) = self.stage_bytes(stored, &body_hash) else {
            return;
        };
        let seq = self.next_seq;
        self.next_seq += 1;
        self.staged.push(BodyIndexRow {
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
            block_offset: reference.block_offset,
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
    fn stage_bytes(&mut self, bytes: &[u8], body_hash: &str) -> Option<BodyRef> {
        if self.block_is_stale() {
            self.close_block();
        }
        if let Some(reference) = self.block_spans.get(body_hash) {
            ::metrics::counter!(DB_ARCHIVE_BODIES_DEDUPLICATED_TOTAL).increment(1);
            return Some(*reference);
        }
        match self.stage_into_open_block(bytes, body_hash)? {
            Ok(reference) => return Some(reference),
            Err(ArchiveError::BlockFull) => {}
            Err(error) => {
                self.refuse_body(&error, bytes.len(), "stage");
                return None;
            }
        }
        self.close_block();
        match self.stage_into_open_block(bytes, body_hash) {
            Some(Ok(reference)) => Some(reference),
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
        if let Some(writer) = self.writer.take() {
            writer.abandon();
        }
    }

    /// The archive file this session writes, when it has one and the writer
    /// is still in service. `None` is retention's cue that there is nothing
    /// it may rewrite: an in-memory ledger has no file, and an archive that
    /// gave up must not have its file compacted underneath index rows it can
    /// no longer vouch for.
    pub(super) fn path_in_service(&self) -> Option<&Path> {
        self.writer.as_ref().and(self.path.as_deref())
    }

    /// Reopen the writer after retention rewrote the file.
    ///
    /// The old writer's `end` is the old file's length, so appending through
    /// it would write past -- or, after a compaction, on top of -- blocks the
    /// index still names. The reopened writer reads the compacted length back
    /// from the file, which is the only place it is now true.
    ///
    /// Callable only with nothing in hand: retention closes and commits
    /// first, so an open block or a row waiting to commit would mean the
    /// sequence was not followed.
    pub(super) fn reopen_after_retention(&mut self) {
        debug_assert!(
            !self.has_work() && self.block_opened_at.is_none(),
            "retention reopened the archive with {} segments and {} rows still in hand",
            self.appended.len(),
            self.staged.len()
        );
        self.writer = self.path.as_deref().and_then(open_writer);
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
        for (_, rows) in &mut self.appended {
            rows.retain(|row| row.seq < mark);
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
        !self.appended.is_empty() || !self.staged.is_empty() || self.pending_bytes() > 0
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
                self.appended.push((segment, rows));
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

    /// Record every written segment's block and the index rows it holds.
    ///
    /// Runs inside the caller's transaction, after `flush_segment` put the
    /// bytes in the file. Nothing here can make a row visible before its
    /// bytes.
    ///
    /// The archive is flushed to the device first. The segment write only
    /// went through the page cache, so without this a power loss could leave
    /// SQLite's durably-committed index rows naming segments that never
    /// reached the disk. The cost is one `fdatasync` per flush that has
    /// segments to index -- at most once per flush interval, never once per
    /// body. A failed flush means those bytes may not be there, so their rows
    /// are dropped and archiving stops. Losing bodies is recoverable; an index
    /// that lies is not.
    ///
    /// `body_blocks` holds one row per block, and each segment grows it: the
    /// upsert records the block's committed raw length and on-disk extent,
    /// and never shrinks them, so a flush retried after a rollback is the
    /// same as the first. Not `REPLACE`: that is a delete and an insert, which
    /// under enforced foreign keys would take every index row naming the block
    /// with it. The blocks are all written before any row, so a row always
    /// names a block the transaction already holds.
    ///
    /// The segments stay in hand until the caller says the transaction
    /// committed. An insert here can fail, and so can the memory-table copy
    /// beside it and the commit after it; all three roll back every row, and
    /// the next flush retries these with them.
    pub(super) fn commit_index_rows(&mut self, conn: &Connection) -> rusqlite::Result<()> {
        if self.appended.is_empty() {
            return Ok(());
        }
        if !self.sync_appended_segments() {
            return Ok(());
        }
        // Recorded here, where the first row is actually written, and not
        // beside the flush that precedes it: a trace whose two steps are
        // pushed from one place proves their order only to itself.
        #[cfg(test)]
        self.steps.push("commit");
        let sealed_at = format_timestamp((self.now)());
        for (segment, _) in &self.appended {
            upsert_block(conn, segment, &sealed_at)?;
        }
        for row in self.appended.iter().flat_map(|(_, rows)| rows) {
            execute_cached(
                conn,
                "INSERT OR REPLACE INTO event_body_blobs (
                    event_id, event_type, source_table, direction, content_type,
                    original_bytes, stored_bytes, truncated, body_hash,
                    block_offset, body_offset, body_len,
                    trace_id, turn_id, created_at
                 )
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                params![
                    &row.event_id,
                    row.event_type,
                    row.source_table,
                    row.direction,
                    &row.content_type,
                    row.original_bytes,
                    row.stored_bytes,
                    i64::from(row.truncated),
                    &row.body_hash,
                    row.block_offset as i64,
                    row.body_offset,
                    row.stored_bytes,
                    &row.trace_id,
                    &row.turn_id,
                    &row.created_at,
                ],
            )?;
        }
        Ok(())
    }

    /// The transaction that `commit_index_rows` wrote into has committed, so
    /// those segments are indexed and this archive is done with them.
    pub(super) fn index_rows_committed(&mut self) {
        self.appended.clear();
    }

    /// Flush the written segments to the device before their rows commit.
    /// `false` when the flush failed and the rows must not be written.
    fn sync_appended_segments(&mut self) -> bool {
        let Some(writer) = self.writer.as_mut() else {
            // The writer is already gone -- an earlier write or flush gave
            // up -- so these segments will never be vouched for. Dropping them
            // is the same fail-closed answer, and keeping them would leave
            // every later flush opening a transaction to do nothing with.
            self.abandon_uncommitted("poisoned");
            return false;
        };
        if let Err(error) = writer.sync() {
            warn!(
                error = %error,
                dropped_segments = self.appended.len(),
                "session body archive could not be flushed; no further bodies will be stored"
            );
            self.abandon_uncommitted("sync");
            self.retire_writer();
            return false;
        }
        #[cfg(test)]
        self.steps.push("sync");
        true
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
        let dropped: usize = self.appended.iter().map(|(_, rows)| rows.len()).sum::<usize>() + self.staged.len();
        self.appended.clear();
        self.staged.clear();
        self.drop_bodies(dropped, reason);
    }

    /// Count bodies the archive gave up on. A session whose archive poisoned
    /// itself keeps serving every other ledger row, so nothing downstream
    /// looks wrong; this counter is how that shows up anywhere but a log line.
    fn drop_bodies(&self, count: usize, reason: &'static str) {
        if count == 0 {
            return;
        }
        ::metrics::counter!(super::DB_ARCHIVE_BODIES_DROPPED_TOTAL, "reason" => reason).increment(count as u64);
    }

    /// Whether a test asked this archive to close its block at every flush.
    fn closes_at_every_flush(&self) -> bool {
        #[cfg(test)]
        if let Some(path) = &self.path {
            return CLOSE_AT_EVERY_FLUSH
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .contains(path);
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
        self.appended.len()
    }

    /// The flush/commit steps this archive has taken, in order.
    #[cfg(test)]
    pub(super) fn steps_for_tests(&self) -> &[&'static str] {
        &self.steps
    }
}

/// Record one written segment against its block: insert the block's row on
/// its first segment, grow it on every later one.
///
/// `WHERE excluded.disk_len >= disk_len` keeps it monotone, so a retried
/// flush that replays an older segment after a newer one committed cannot
/// shrink the extent a reader or retention relies on. A retry that finds a
/// *larger* extent recorded is the one case that should never happen, and
/// is said out loud.
///
/// `sealed_at` moves only when the segment brought bodies. Retention cuts by
/// it, and it closes the open block before it looks: an empty FINAL segment
/// stamped "now" would make every idle block the newest thing in the archive
/// at the very moment retention asked how old it was, and nothing would ever
/// age out of a block that was open when retention ran.
fn upsert_block(conn: &Connection, segment: &SegmentWritten, sealed_at: &str) -> rusqlite::Result<()> {
    let recorded = execute_cached(
        conn,
        "INSERT INTO body_blocks (block_offset, raw_len, disk_len, sealed_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(block_offset) DO UPDATE SET
            raw_len = excluded.raw_len,
            disk_len = excluded.disk_len,
            sealed_at = CASE WHEN excluded.raw_len > body_blocks.raw_len
                             THEN excluded.sealed_at ELSE body_blocks.sealed_at END
         WHERE excluded.disk_len >= body_blocks.disk_len",
        params![
            segment.block_offset as i64,
            i64::from(segment.raw_len),
            segment.disk_len as i64,
            sealed_at,
        ],
    )?;
    if recorded == 0 {
        warn!(
            block_offset = segment.block_offset,
            raw_len = segment.raw_len,
            disk_len = segment.disk_len,
            "session body index already records a longer extent for this block; kept the longer one"
        );
    }
    Ok(())
}
