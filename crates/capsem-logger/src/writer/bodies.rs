//! The writer thread's half of the body archive.
//!
//! Bodies are staged into a pending block as their event is written, and the
//! index rows that name them are held beside the block they belong to. The
//! order is the whole point: a block's bytes are appended to the archive file
//! *before* the transaction that inserts its `body_blocks` row and its index
//! rows commits. A crash between the two leaves an unreferenced block at the
//! end of the file, which costs its bytes and nothing else; the reverse order
//! would leave index rows pointing past EOF, which is a ledger that lies.
//!
//! Compression runs on this thread, synchronously, through the archive's
//! two-phase API (`take_pending` / `encode` / `append`). Deflating 256 KiB
//! costs a few milliseconds once per block, and keeping it here means the
//! append order is the take order by construction rather than by protocol.
//! Moving `encode` onto its own thread stays a local change to `seal_pending`.

use std::path::{Path, PathBuf};

use capsem_archive::{ArchiveError, BodyLogWriter, BodyRef, SealedBlock, BLOCK_HEADER_BYTES};
use rusqlite::{params, Connection};
use tracing::warn;

use super::{blake3_bytes_ref, execute_cached, format_timestamp, LedgerClock, MAX_BODY_BLOB_BYTES};

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

/// An index row waiting for its block's bytes to reach the archive file.
struct BodyIndexRow {
    /// Monotonic across the archive, so a rolled-back transaction can drop
    /// its own rows wherever they ended up -- including a block that sealed
    /// while the transaction was still open.
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
    /// beside, and after an append failure, which makes every later offset
    /// unprovable. Both stage nothing rather than writing an index nobody can
    /// resolve.
    writer: Option<BodyLogWriter>,
    /// What `created_at` and `sealed_at` are read from. Every other column in
    /// the index is content, so this is the only value in it that a replay of
    /// the same session cannot reproduce -- and the only reason the fixture
    /// regenerator needs to supply its own.
    now: LedgerClock,
    /// Rows for the block currently being staged.
    staged: Vec<BodyIndexRow>,
    /// Sequence number the next staged row takes.
    next_seq: u64,
    /// Blocks whose bytes are on disk, waiting for their index rows to commit.
    appended: Vec<(SealedBlock, Vec<BodyIndexRow>)>,
    /// Test builds only: make the next append fail the way a full disk does.
    /// Injected at this seam rather than inside the archive writer because
    /// what is under test is what *this* type does once the writer is gone.
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

/// Whether every block the index names is inside the archive file.
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
/// refusing costs only this session's new bodies. A false answer here is
/// treated as a false one: a `body_blocks` that cannot be read is broken
/// schema, not an empty table.
fn index_fits_the_file(conn: &Connection, archive_path: &Path) -> bool {
    let indexed_end: Option<i64> =
        match conn.query_row("SELECT MAX(block_offset + comp_len) FROM body_blocks", [], |row| {
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
    let indexed_end = indexed_end.saturating_add(BLOCK_HEADER_BYTES as i64);
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
            staged: Vec::new(),
            next_seq: 0,
            appended: Vec::new(),
            #[cfg(test)]
            fail_next_append: false,
            #[cfg(test)]
            steps: Vec::new(),
        }
    }

    /// Stage one body: its bytes into the pending block, its index row beside
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
        let Some(reference) = self.stage_bytes(&bytes[..stored_len]) else {
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
            truncated: original_bytes > stored_len as u64,
            // Over the stored bytes, not the buffer they were cut from: a
            // hash of something the archive does not hold cannot be checked
            // against anything, and a reader that verifies what it read is
            // how a corrupted index row stops being a body.
            body_hash: blake3_bytes_ref(&bytes[..stored_len]),
            body_offset: i64::from(reference.offset),
            trace_id: blob.trace_id.map(str::to_string),
            turn_id: blob.turn_id.map(str::to_string),
            created_at: format_timestamp((self.now)()),
        });
    }

    /// Stage bytes into the pending block, sealing first if that block cannot
    /// hold them.
    ///
    /// `BlockFull` is the archive's seal-and-retry contract, not a failure:
    /// the pending block is within 16 MiB of its ceiling and this body does
    /// not fit beside what is already there. Sealing empties it, and a body
    /// capped at `MAX_BODY_BLOB_BYTES` (10 MiB) always fits an empty one, so
    /// the retry is the last step and not a loop.
    ///
    /// Every other error skips the body and logs. `BodyTooLarge` cannot
    /// happen at this cap; a poisoned writer takes the archive out of service
    /// here, because its pending block can no longer be placed and the rows
    /// staged beside it would name offsets in a file that never received
    /// them. In neither case may a body take down the writer thread that owns
    /// the whole session ledger.
    fn stage_bytes(&mut self, bytes: &[u8]) -> Option<BodyRef> {
        match self.writer.as_mut()?.stage(bytes) {
            Ok(reference) => return Some(reference),
            Err(ArchiveError::BlockFull) => {}
            Err(error) => {
                warn!(error = %error, body_bytes = bytes.len(), "body not archived");
                if takes_the_archive_out_of_service(&error) {
                    self.give_up("stage");
                }
                self.drop_bodies(1, "stage");
                return None;
            }
        }
        self.seal_pending();
        let Some(writer) = self.writer.as_mut() else {
            // `seal_pending` failed its append and retired the writer. The
            // count it took covered the rows it was holding; this body has no
            // row yet, so it was not among them. Without this line it is the
            // one body the counter never sees.
            warn!(body_bytes = bytes.len(), "body not archived after seal");
            self.drop_bodies(1, "stage_after_seal");
            return None;
        };
        match writer.stage(bytes) {
            Ok(reference) => Some(reference),
            Err(error) => {
                warn!(error = %error, body_bytes = bytes.len(), "body not archived after seal");
                if takes_the_archive_out_of_service(&error) {
                    self.give_up("stage_after_seal");
                }
                self.drop_bodies(1, "stage_after_seal");
                None
            }
        }
    }

    /// Take the archive out of service, dropping the rows it can no longer
    /// place.
    ///
    /// A poisoned writer refuses every later call, so the rows staged against
    /// its pending block will never have bytes to point at. Keeping them would
    /// leave the next flush inserting index rows for bytes no file received,
    /// which is the one outcome the whole split exists to prevent.
    pub(super) fn give_up(&mut self, reason: &'static str) {
        // Seal first so a writer that could still place bytes is not dropped
        // holding a block. A poisoned one hands out nothing here, which is
        // the case this exists for; anything else leaves the file tidy and
        // the bytes merely unreferenced.
        self.seal_pending();
        self.abandon_uncommitted(reason);
        self.writer = None;
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
    /// Callable only with nothing in hand: retention flushes first, so a block
    /// waiting for its index row would mean the sequence was not followed.
    pub(super) fn reopen_after_retention(&mut self) {
        debug_assert!(
            !self.has_work(),
            "retention reopened the archive with {} blocks and {} rows still in hand",
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
    /// unreferenced, and cheaper than rewriting every later offset. Rows of a
    /// block that sealed mid-transaction are dropped with the rest.
    pub(super) fn rollback_staged(&mut self, mark: u64) {
        self.staged.retain(|row| row.seq < mark);
        for (_, rows) in &mut self.appended {
            rows.retain(|row| row.seq < mark);
        }
    }

    /// Seal the pending block if it is full. Called between the operations of
    /// a batch as well as after it, so one large batch cannot hold an
    /// unbounded pile of raw bodies in RAM until the batch ends.
    pub(super) fn seal_if_full(&mut self) {
        if self.wants_seal() {
            self.seal_pending();
        }
    }

    pub(super) fn pending_bytes(&self) -> usize {
        self.writer.as_ref().map_or(0, BodyLogWriter::pending_bytes)
    }

    /// True once the pending block has reached the archive's target size, so
    /// a burst of large bodies seals without waiting for the flush interval.
    pub(super) fn wants_seal(&self) -> bool {
        self.writer.as_ref().is_some_and(BodyLogWriter::wants_seal)
    }

    /// Whether a flush has anything to do here: rows waiting to commit, or
    /// bytes waiting to seal. Pending bytes count even when their rows were
    /// dropped by a rollback -- the block still has to reach the file and be
    /// let go of, or the writer closes holding it.
    pub(super) fn has_work(&self) -> bool {
        !self.appended.is_empty() || !self.staged.is_empty() || self.pending_bytes() > 0
    }

    /// Deflate the pending block and append it to the archive file. Its index
    /// rows move with it and wait for the next transaction.
    pub(super) fn seal_pending(&mut self) {
        // Taken before the writer is borrowed, and only in test builds.
        #[cfg(test)]
        let injected_append_failure = std::mem::take(&mut self.fail_next_append);
        let Some(writer) = self.writer.as_mut() else {
            return;
        };
        let Some(pending) = writer.take_pending() else {
            return;
        };
        let rows = std::mem::take(&mut self.staged);
        #[cfg(test)]
        let appended = if injected_append_failure {
            Err(ArchiveError::Io(std::io::Error::other("injected append failure")))
        } else {
            writer.append(pending.encode())
        };
        #[cfg(not(test))]
        let appended = writer.append(pending.encode());
        match appended {
            Ok(sealed) => self.appended.push((sealed, rows)),
            Err(error) => {
                // The file's end is no longer provably where the writer
                // believes it is, so every later offset would be a guess.
                // Stop archiving rather than index bytes we cannot name.
                warn!(
                    error = %error,
                    dropped_bodies = rows.len(),
                    "session body archive append failed; no further bodies will be stored"
                );
                self.drop_bodies(rows.len(), "append");
                self.writer = None;
            }
        }
    }

    /// Insert every appended block and the index rows it holds.
    ///
    /// Runs inside the caller's transaction, after `seal_pending` put the
    /// bytes on disk. Nothing here can make a row visible before its bytes.
    ///
    /// The archive is flushed to the device first. `append` only wrote
    /// through the page cache, so without this a power loss could leave
    /// SQLite's durably-committed index rows naming blocks that never reached
    /// the disk -- a ledger that points past its own file, which is the one
    /// failure this ordering exists to prevent. The cost is one `fdatasync`
    /// per flush that has blocks to index -- at most once per flush interval,
    /// covering every block sealed since the last one, never once per body.
    ///
    /// A failed flush means those bytes may not be there, so their rows are
    /// dropped and archiving stops, exactly as a failed append does. Losing
    /// bodies is recoverable; an index that lies is not.
    ///
    /// The blocks stay in hand until the caller says the transaction
    /// committed. An insert here can fail, and so can the memory-table copy
    /// beside it and the commit after it; all three roll back every row. The
    /// dirty tables are kept for the next flush to retry, and these blocks
    /// are retried with them -- `INSERT OR REPLACE` makes the second attempt
    /// the same as the first.
    pub(super) fn commit_index_rows(&mut self, conn: &Connection) -> rusqlite::Result<()> {
        if self.appended.is_empty() {
            return Ok(());
        }
        if !self.sync_appended_blocks() {
            return Ok(());
        }
        // Recorded here, where the first row is actually written, and not
        // beside the flush that precedes it: a trace whose two steps are
        // pushed from one place proves their order only to itself.
        #[cfg(test)]
        self.steps.push("commit");
        for (block, rows) in &self.appended {
            // IGNORE, not REPLACE, and defensive rather than load-bearing: no
            // path known today reaches this insert with the row already there,
            // because the retry after a rolled-back flush re-inserts a row the
            // rollback took away. It is IGNORE because a block row is
            // immutable once written, so the only correct answer to finding
            // one is to leave it alone -- and because REPLACE is a delete
            // followed by an insert, which would take every index row
            // referencing the block with it under enforced foreign keys.
            let inserted = execute_cached(
                conn,
                "INSERT OR IGNORE INTO body_blocks (block_offset, raw_len, comp_len, sealed_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    block.block_offset as i64,
                    i64::from(block.raw_len),
                    i64::from(block.comp_len),
                    format_timestamp((self.now)()),
                ],
            )?;
            if inserted == 0 {
                // IGNORE is what keeps a retried flush idempotent, and it is
                // also the one way a *new* block could be filed under a row
                // describing a different one -- its index rows would then
                // resolve against the wrong bytes. Nothing reaches this
                // today; saying so out loud is what stops it being silent if
                // something ever does.
                warn!(
                    block_offset = block.block_offset,
                    raw_len = block.raw_len,
                    comp_len = block.comp_len,
                    "session body index already holds a block at this offset; its rows may name \
                     bytes that belong to another block"
                );
            }
            for row in rows.iter() {
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
                        block.block_offset as i64,
                        row.body_offset,
                        row.stored_bytes,
                        &row.trace_id,
                        &row.turn_id,
                        &row.created_at,
                    ],
                )?;
            }
        }
        Ok(())
    }

    /// The transaction that `commit_index_rows` wrote into has committed, so
    /// those blocks are indexed and this archive is done with them.
    pub(super) fn index_rows_committed(&mut self) {
        self.appended.clear();
    }

    /// Flush the appended blocks to the device before their rows commit.
    /// `false` when the flush failed and the rows must not be written.
    fn sync_appended_blocks(&mut self) -> bool {
        let Some(writer) = self.writer.as_mut() else {
            // The writer is already gone -- an earlier append or flush gave
            // up -- so these blocks will never be vouched for. Dropping them
            // is the same fail-closed answer, and keeping them would leave
            // every later flush opening a transaction to do nothing with.
            self.abandon_uncommitted("poisoned");
            return false;
        };
        if let Err(error) = writer.sync() {
            warn!(
                error = %error,
                dropped_blocks = self.appended.len(),
                "session body archive could not be flushed; no further bodies will be stored"
            );
            self.abandon_uncommitted("sync");
            self.writer = None;
            return false;
        }
        #[cfg(test)]
        self.steps.push("sync");
        true
    }

    /// Durability barrier for session close.
    ///
    /// Seals first, unconditionally: whatever is still pending has no index
    /// row left that could reference it -- a flush would have taken it
    /// otherwise -- but the block still has to leave the writer, which closes
    /// holding nothing. An unreferenced block at the end of the file is the
    /// documented cost; a writer dropped with bodies in hand is a bug.
    pub(super) fn sync(&mut self) {
        self.seal_pending();
        if let Some(writer) = self.writer.as_mut() {
            if let Err(error) = writer.sync() {
                warn!(error = %error, "session body archive sync failed");
            }
        }
    }

    /// Give up on every body this archive is still holding a row for, counting
    /// them under `reason`. Both halves go: a block waiting to be indexed and
    /// a pending block's rows are equally unreachable once the archive stops
    /// vouching for its own file.
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

    /// Put this archive in the state a failed append leaves: no writer, and
    /// whatever blocks had already been appended still waiting for their rows.
    #[cfg(test)]
    pub(super) fn poison_writer_for_tests(&mut self) {
        self.writer = None;
    }

    /// Make the next `seal_pending` fail its append, as a full disk would.
    #[cfg(test)]
    pub(super) fn fail_next_append_for_tests(&mut self) {
        self.fail_next_append = true;
    }

    /// Blocks appended but not yet indexed.
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
