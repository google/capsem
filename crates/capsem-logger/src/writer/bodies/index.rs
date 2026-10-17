//! SQLite publication of durable archive segments and body index rows.

use super::*;
use rusqlite::{params_from_iter, ToSql};

const BODY_INDEX_ROWS_PER_INSERT: usize = 64;
const BODY_INDEX_VALUE_LIST: &str = "(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";

impl BodyArchive {
    /// Record every durably-written segment's block and the index rows it holds.
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
    pub(in crate::writer) fn commit_index_rows(&mut self, conn: &Connection) -> rusqlite::Result<()> {
        self.prepare_index_rows(true);
        self.commit_prepared_index_rows_with_limit(conn, usize::MAX)
    }

    pub(in crate::writer) fn commit_prepared_index_rows_with_limit(
        &mut self,
        conn: &Connection,
        max_rows: usize,
    ) -> rusqlite::Result<()> {
        self.prepared_index_rows = None;
        if self.synced.is_empty() && self.reused.is_empty() {
            return Ok(());
        }
        // Recorded here, where the first row is actually written, and not
        // beside the flush that precedes it: a trace whose two steps are
        // pushed from one place proves their order only to itself.
        #[cfg(test)]
        self.steps.push("commit");
        let synced_rows = self.synced.iter().map(|segment| segment.rows.len()).sum::<usize>();
        let mut prepared = PreparedIndexRows {
            synced: synced_rows.min(max_rows),
            reused: self.reused.len().min(max_rows.saturating_sub(synced_rows)),
            blocks: Vec::new(),
        };
        let sealed_at = format_timestamp((self.now)());
        let mut committed_end = None;
        for (index, queued) in self.synced.iter().enumerate() {
            // A zero-row FINAL segment still advances the durable extent. All
            // newly synced block metadata is therefore published once even
            // when this transaction takes only a bounded prefix of body rows.
            if queued.block_committed {
                continue;
            }
            upsert_block(conn, &queued.segment, &sealed_at)?;
            prepared.blocks.push(index);
            let end = queued
                .segment
                .block_offset
                .checked_add(queued.segment.disk_len)
                .ok_or_else(|| {
                    rusqlite::Error::InvalidParameterName("archive committed extent overflows u64".into())
                })?;
            committed_end = Some(committed_end.map_or(end, |current: u64| current.max(end)));
        }
        if let Some(committed_end) = committed_end {
            let committed_end = i64::try_from(committed_end).map_err(|_| {
                rusqlite::Error::InvalidParameterName("archive committed extent exceeds SQLite INTEGER".into())
            })?;
            conn.execute(
                "UPDATE archive_state
                 SET committed_end = MAX(committed_end, ?1)
                 WHERE singleton = 1",
                [committed_end],
            )?;
        }
        let rows = self
            .synced
            .iter()
            .flat_map(|segment| &segment.rows)
            .chain(&self.reused)
            .take(prepared.total())
            .collect::<Vec<_>>();
        insert_body_index_rows(conn, &rows)?;
        self.prepared_index_rows = Some(prepared);
        Ok(())
    }

    pub(in crate::writer) fn prepared_index_row_count(&self) -> usize {
        self.prepared_index_rows.as_ref().map_or(0, |prepared| prepared.total())
    }

    /// The transaction that `commit_index_rows` wrote into has committed, so
    /// those segments are indexed and this archive is done with them.
    pub(in crate::writer) fn index_rows_committed(&mut self) {
        let Some(prepared) = self.prepared_index_rows.take() else {
            return;
        };
        for index in prepared.blocks {
            self.synced[index].block_committed = true;
        }
        let mut synced = prepared.synced;
        let mut committed_segments = 0;
        for segment in &mut self.synced {
            let row_count = segment.rows.len();
            if row_count == 0 {
                committed_segments += 1;
            } else if row_count <= synced {
                synced -= row_count;
                committed_segments += 1;
            } else if synced > 0 {
                segment.rows.drain(..synced);
                break;
            } else {
                break;
            }
        }
        self.synced.drain(..committed_segments);
        self.reused.drain(..prepared.reused);
    }
}

fn insert_body_index_rows(conn: &Connection, rows: &[&BodyIndexRow]) -> rusqlite::Result<()> {
    for chunk in rows.chunks(BODY_INDEX_ROWS_PER_INSERT) {
        let mut sql = String::from(
            "INSERT OR REPLACE INTO event_body_blobs (
                event_id, event_type, source_table, direction, content_type,
                original_bytes, stored_bytes, truncated, body_hash,
                block_offset, body_offset, body_len,
                trace_id, turn_id, created_at
             ) VALUES ",
        );
        for index in 0..chunk.len() {
            if index > 0 {
                sql.push_str(", ");
            }
            sql.push_str(BODY_INDEX_VALUE_LIST);
        }

        // References only: the rows keep ownership until the surrounding
        // transaction commits, so batching does not clone any body metadata.
        let mut values: Vec<&dyn ToSql> = Vec::with_capacity(chunk.len() * 15);
        for row in chunk {
            values.extend([
                &row.event_id as &dyn ToSql,
                &row.event_type as &dyn ToSql,
                &row.source_table as &dyn ToSql,
                &row.direction as &dyn ToSql,
                &row.content_type as &dyn ToSql,
                &row.original_bytes as &dyn ToSql,
                &row.stored_bytes as &dyn ToSql,
                &row.truncated as &dyn ToSql,
                &row.body_hash as &dyn ToSql,
                &row.block_offset as &dyn ToSql,
                &row.body_offset as &dyn ToSql,
                &row.stored_bytes as &dyn ToSql,
                &row.trace_id as &dyn ToSql,
                &row.turn_id as &dyn ToSql,
                &row.created_at as &dyn ToSql,
            ]);
        }
        execute_cached(conn, &sql, params_from_iter(values))?;
    }
    Ok(())
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
