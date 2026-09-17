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
use std::time::SystemTime;

use capsem_archive::{ArchiveError, BodyLogWriter, BodyRef, SealedBlock};
use rusqlite::{params, Connection};
use tracing::warn;

use super::{blake3_bytes_ref, execute_cached, format_timestamp, MAX_BODY_BLOB_BYTES};

/// One body to archive, as its `insert_*` writer describes it.
pub(super) struct EventBodyBlob<'a> {
    pub(super) event_id: &'a str,
    pub(super) event_type: &'static str,
    pub(super) source_table: &'static str,
    pub(super) direction: &'static str,
    pub(super) content_type: Option<&'a str>,
    pub(super) body: Option<&'a str>,
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
    /// `None` for an in-memory database, which has no file to put an archive
    /// beside, and after an append failure, which makes every later offset
    /// unprovable. Both stage nothing rather than writing an index nobody can
    /// resolve.
    writer: Option<BodyLogWriter>,
    /// Rows for the block currently being staged.
    staged: Vec<BodyIndexRow>,
    /// Sequence number the next staged row takes.
    next_seq: u64,
    /// Blocks whose bytes are on disk, waiting for their index rows to commit.
    appended: Vec<(SealedBlock, Vec<BodyIndexRow>)>,
    /// "sync" and "commit" in the order they happened, so a test can prove
    /// the archive is flushed before the rows that name it are written.
    #[cfg(test)]
    steps: Vec<&'static str>,
}

/// Where a session's archive lives: `session.bodies` beside `session.db`.
pub(crate) fn archive_path_for_db(db_path: &Path) -> PathBuf {
    db_path.with_extension("bodies")
}

impl BodyArchive {
    pub(super) fn open(db_path: Option<&Path>) -> Self {
        let writer = db_path.and_then(|path| {
            let archive_path = archive_path_for_db(path);
            match BodyLogWriter::open(&archive_path) {
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
        });
        Self {
            writer,
            staged: Vec::new(),
            next_seq: 0,
            appended: Vec::new(),
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
        let Some(body) = blob.body.filter(|body| !body.is_empty()) else {
            return;
        };
        let bytes = body.as_bytes();
        let stored_len = bytes.len().min(MAX_BODY_BLOB_BYTES);
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
            original_bytes: bytes.len() as i64,
            stored_bytes: stored_len as i64,
            truncated: bytes.len() > stored_len,
            body_hash: blake3_bytes_ref(bytes),
            body_offset: i64::from(reference.offset),
            trace_id: blob.trace_id.map(str::to_string),
            turn_id: blob.turn_id.map(str::to_string),
            created_at: format_timestamp(SystemTime::now()),
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
    /// happen at this cap, and a poisoned writer has already warned once; in
    /// neither case may a body take down the writer thread that owns the
    /// whole session ledger.
    fn stage_bytes(&mut self, bytes: &[u8]) -> Option<BodyRef> {
        match self.writer.as_mut()?.stage(bytes) {
            Ok(reference) => return Some(reference),
            Err(ArchiveError::BlockFull) => {}
            Err(error) => {
                warn!(error = %error, body_bytes = bytes.len(), "body not archived");
                return None;
            }
        }
        self.seal_pending();
        match self.writer.as_mut()?.stage(bytes) {
            Ok(reference) => Some(reference),
            Err(error) => {
                warn!(error = %error, body_bytes = bytes.len(), "body not archived after seal");
                None
            }
        }
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

    pub(super) fn has_uncommitted_rows(&self) -> bool {
        !self.appended.is_empty() || !self.staged.is_empty()
    }

    /// Deflate the pending block and append it to the archive file. Its index
    /// rows move with it and wait for the next transaction.
    pub(super) fn seal_pending(&mut self) {
        let Some(writer) = self.writer.as_mut() else {
            return;
        };
        let Some(pending) = writer.take_pending() else {
            return;
        };
        let rows = std::mem::take(&mut self.staged);
        match writer.append(pending.encode()) {
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
    /// per sealed block: once per 256 KiB of bodies, or once per flush
    /// interval, not once per body.
    ///
    /// A failed flush means those bytes may not be there, so their rows are
    /// dropped and archiving stops, exactly as a failed append does. Losing
    /// bodies is recoverable; an index that lies is not.
    pub(super) fn commit_index_rows(&mut self, conn: &Connection) -> rusqlite::Result<()> {
        if self.appended.is_empty() {
            return Ok(());
        }
        if !self.sync_appended_blocks() {
            return Ok(());
        }
        for (block, rows) in std::mem::take(&mut self.appended) {
            execute_cached(
                conn,
                "INSERT OR REPLACE INTO body_blocks (block_offset, raw_len, comp_len, sealed_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    block.block_offset as i64,
                    i64::from(block.raw_len),
                    i64::from(block.comp_len),
                    format_timestamp(SystemTime::now()),
                ],
            )?;
            for row in rows {
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
                        row.event_id,
                        row.event_type,
                        row.source_table,
                        row.direction,
                        row.content_type,
                        row.original_bytes,
                        row.stored_bytes,
                        i64::from(row.truncated),
                        row.body_hash,
                        block.block_offset as i64,
                        row.body_offset,
                        row.stored_bytes,
                        row.trace_id,
                        row.turn_id,
                        row.created_at,
                    ],
                )?;
            }
        }
        Ok(())
    }

    /// Flush the appended blocks to the device before their rows commit.
    /// `false` when the flush failed and the rows must not be written.
    fn sync_appended_blocks(&mut self) -> bool {
        #[cfg(test)]
        self.steps.push("sync");
        let Some(writer) = self.writer.as_mut() else {
            return false;
        };
        if let Err(error) = writer.sync() {
            warn!(
                error = %error,
                dropped_blocks = self.appended.len(),
                "session body archive could not be flushed; no further bodies will be stored"
            );
            self.appended.clear();
            self.writer = None;
            return false;
        }
        #[cfg(test)]
        self.steps.push("commit");
        true
    }

    /// Durability barrier for session close.
    pub(super) fn sync(&mut self) {
        if let Some(writer) = self.writer.as_mut() {
            if let Err(error) = writer.sync() {
                warn!(error = %error, "session body archive sync failed");
            }
        }
    }

    /// The flush/commit steps this archive has taken, in order.
    #[cfg(test)]
    pub(super) fn steps_for_tests(&self) -> &[&'static str] {
        &self.steps
    }
}
