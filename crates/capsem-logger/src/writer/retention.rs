//! Dropping archived bodies whose blocks have aged past a cutoff.
//!
//! Retention runs on the writer thread, because `capsem-process` owns every
//! write to `session.db` and `session.bodies` and this rewrites both. The
//! service holds external, disk-only readers and must ask rather than act --
//! see `DbHandle::retain_bodies_since`, which refuses a handle with no writer.
//!
//! The unit is a block, not a body. A block is the smallest thing the archive
//! can drop without re-deflating what survives, and `body_blocks.sealed_at`
//! is when its bytes were written, so "older than the cutoff" is a property
//! the index already records.
//!
//! Order of operations, and why:
//!
//! 1. Everything staged is sealed and its index rows committed first. A block
//!    whose rows have not committed is invisible to the keep query, so it
//!    would be compacted away and its rows inserted afterwards pointing at
//!    offsets that no longer exist.
//! 2. `retain_blocks` rewrites the file. It is atomic -- a temporary renamed
//!    over the original -- so the file is either wholly the old one or wholly
//!    the new one.
//! 3. One transaction deletes the rows of the dropped blocks and remaps the
//!    survivors' offsets.
//!
//! Between 2 and 3 the index names offsets the file no longer has. That
//! window is a SQLite transaction wide, and it is not silent: every body read
//! verifies blake3 over the bytes the row's span selected, so a stale offset
//! fails the read loudly instead of returning some other body's bytes. A
//! failure in step 3 leaves the ledger in exactly that state, which is why it
//! is reported as an error rather than swallowed.

use std::collections::BTreeMap;

use rusqlite::{params, Connection};

use super::bodies::BodyArchive;

/// What one retention pass did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RetainOutcome {
    /// Blocks whose bytes left the archive file.
    pub blocks_dropped: u64,
    /// Blocks that survived and were remapped.
    pub blocks_kept: u64,
    /// `event_body_blobs` rows deleted with their blocks.
    pub rows_dropped: u64,
    /// How much shorter `session.bodies` is.
    pub bytes_reclaimed: u64,
}

/// Drop every archived block sealed before `cutoff` (RFC 3339), compacting the
/// archive file and rewriting the index rows that name it.
///
/// # Errors
///
/// A string, because every caller of this is already on the `DbResult` rail.
/// The archive being closed, unflushed work still in hand, a block the file
/// cannot produce, and any SQLite failure all surface here; none of them
/// leave a body readable that should have gone, and none of them delete an
/// index row whose block is still in the file.
pub(super) fn retain_bodies(
    conn: &Connection,
    bodies: &mut BodyArchive,
    cutoff: &str,
) -> Result<RetainOutcome, String> {
    if bodies.has_work() {
        return Err("session body retention needs a flushed archive; blocks are still waiting for index rows".into());
    }
    let path = bodies
        .path_in_service()
        .ok_or("session body archive is not open; nothing was retained")?
        .to_path_buf();

    let before = std::fs::metadata(&path)
        .map_err(|error| format!("session body archive {} could not be measured: {error}", path.display()))?
        .len();
    let keep = kept_block_offsets(conn, cutoff)?;
    let moved = capsem_archive::retain_blocks(&path, &keep).map_err(|error| {
        format!(
            "session body archive {} could not be compacted: {error}",
            path.display()
        )
    })?;
    // The writer's idea of the file's end is now the old length; the next
    // append would land on top of a block the index still names.
    bodies.reopen_after_retention();
    let after = std::fs::metadata(&path)
        .map_err(|error| format!("session body archive {} could not be measured: {error}", path.display()))?
        .len();

    let dropped = reindex(conn, cutoff, &moved).map_err(|error| {
        format!(
            "session body archive {} was compacted but its index could not be rewritten, \
             so body reads will fail their integrity check until it is: {error}",
            path.display()
        )
    })?;
    Ok(RetainOutcome {
        blocks_dropped: dropped.blocks,
        blocks_kept: moved.len() as u64,
        rows_dropped: dropped.rows,
        bytes_reclaimed: before.saturating_sub(after),
    })
}

/// What the deletes removed.
struct Dropped {
    blocks: u64,
    rows: u64,
}

fn kept_block_offsets(conn: &Connection, cutoff: &str) -> Result<Vec<u64>, String> {
    let mut statement = conn
        .prepare_cached("SELECT block_offset FROM body_blocks WHERE sealed_at >= ?1 ORDER BY block_offset")
        .map_err(|error| format!("session body retention could not read body_blocks: {error}"))?;
    let rows = statement
        .query_map(params![cutoff], |row| row.get::<_, i64>(0))
        .map_err(|error| format!("session body retention could not read body_blocks: {error}"))?;
    let mut keep = Vec::new();
    for row in rows {
        let offset = row.map_err(|error| format!("session body retention could not read body_blocks: {error}"))?;
        keep.push(
            u64::try_from(offset)
                .map_err(|_| format!("body_blocks holds a negative block offset {offset}; the index is corrupt"))?,
        );
    }
    Ok(keep)
}

/// Delete the aged-out rows and move the survivors to their new offsets, in
/// one transaction.
///
/// The deletes use the same `sealed_at` predicate the keep set came from
/// rather than a list of offsets: the writer thread is the only writer and is
/// the one running this, so the two evaluations cannot disagree -- and the
/// count check below refuses to go on if they somehow did.
///
/// **Offsets are remapped in ascending order of their old value, and that is
/// what makes a collision impossible.** Compaction only ever moves a block
/// down or leaves it, and it preserves order, so writing `o1 -> n1`, `o2 ->
/// n2`, ... in ascending order can never land on an offset that is still
/// occupied: every already-written `n` is smaller than the `n` being written,
/// and every not-yet-moved `o` is larger than the `o` being written, which is
/// itself at least as large as its `n`. A collision would therefore be a bug
/// in that reasoning, not a case to handle -- and `block_offset` is
/// `body_blocks`' primary key, so `UPDATE` raises a constraint failure and
/// rolls the whole transaction back. Nothing here is an upsert; two blocks
/// can never be quietly merged into one row.
fn reindex(conn: &Connection, cutoff: &str, moved: &BTreeMap<u64, u64>) -> rusqlite::Result<Dropped> {
    let tx = conn.unchecked_transaction()?;
    // The index rows reference `body_blocks(block_offset)`. Remapping moves
    // parent and child one pair at a time, so the two are briefly out of step
    // even though the committed state is consistent. Deferring makes that
    // correct under enforced foreign keys rather than correct only because
    // this ledger does not enable them.
    tx.execute_batch("PRAGMA defer_foreign_keys = ON")?;
    let rows = tx.execute(
        "DELETE FROM event_body_blobs
         WHERE block_offset IN (SELECT block_offset FROM body_blocks WHERE sealed_at < ?1)",
        params![cutoff],
    )?;
    let blocks = tx.execute("DELETE FROM body_blocks WHERE sealed_at < ?1", params![cutoff])?;

    let survivors: i64 = tx.query_row("SELECT COUNT(*) FROM body_blocks", [], |row| row.get(0))?;
    if survivors != moved.len() as i64 {
        // The file was compacted to hold exactly `moved`, so an index that
        // disagrees would leave rows naming blocks that are no longer there.
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "retention kept {} blocks in the archive but {survivors} in the index",
            moved.len()
        )));
    }

    for (old, new) in moved {
        if old == new {
            continue;
        }
        let (old, new) = (*old as i64, *new as i64);
        tx.execute(
            "UPDATE body_blocks SET block_offset = ?2 WHERE block_offset = ?1",
            params![old, new],
        )?;
        tx.execute(
            "UPDATE event_body_blobs SET block_offset = ?2 WHERE block_offset = ?1",
            params![old, new],
        )?;
    }
    tx.commit()?;
    Ok(Dropped {
        blocks: blocks as u64,
        rows: rows as u64,
    })
}
