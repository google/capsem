//! Dropping archived bodies whose blocks have aged past a cutoff.
//!
//! Retention runs on the writer thread, because `capsem-process` owns every
//! write to `session.db` and the archive beside it and this rewrites both. The
//! service holds external, disk-only readers and must ask rather than act --
//! see `DbHandle::retain_bodies_since`, which refuses a handle with no writer.
//!
//! The unit is a block, not a body. A block is the smallest thing the archive
//! can drop without re-deflating what survives, and `body_blocks.sealed_at`
//! is when its last segment was written, so "older than the cutoff" is a
//! property the index already records. A kept block is copied to its
//! committed extent, `body_blocks.disk_len`, and no further.
//!
//! Order of operations, and why:
//!
//! 1. The open block is closed and every index row committed first. A block
//!    whose rows have not committed is invisible to the keep query, so it
//!    would be compacted away and its rows inserted afterwards pointing at
//!    offsets that no longer exist; and a block left open would have its
//!    next segment appended to a file that no longer ends where the writer
//!    believes it does.
//! 2. The compacted archive is **staged**: written and flushed beside the
//!    original, which stays the live file.
//! 3. One transaction deletes the rows of the dropped blocks and remaps the
//!    survivors' offsets, and commits.
//! 4. Only then is the staged file renamed over the original.
//!
//! The index and the file cannot commit together, so the question is not
//! whether a window exists but how wide it is and which way it falls. Between
//! 3 and 4 it is a single `rename(2)`; the old ordering -- rename first, then
//! the transaction -- made it a whole transaction, most of which is the index
//! work itself.
//!
//! And within this process the window closes rather than merely being narrow.
//! A rename that fails leaves the file holding the old offsets, so the
//! remap is reversed and every surviving body is readable again at the offset
//! it always had. What is not recoverable is a crash in that one syscall's
//! width, and even then nothing is served wrongly: every body read verifies
//! blake3 over the bytes its row's span selected, so a stale offset fails the
//! read loudly instead of returning some other body's bytes.

use std::collections::BTreeMap;

use rusqlite::{params, Connection};

use super::bodies::BodyArchive;
use super::retention_faults::{take_retention_failure_for_tests, RetentionFault};

/// What one retention pass did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RetainOutcome {
    /// Blocks whose bytes left the archive file.
    pub blocks_dropped: u64,
    /// Blocks that survived and were remapped.
    pub blocks_kept: u64,
    /// `event_body_blobs` rows deleted with their blocks.
    pub rows_dropped: u64,
    /// How much shorter the archive file is.
    pub bytes_reclaimed: u64,
}

/// Drop every archived block last written before `cutoff` (RFC 3339), compacting the
/// archive file and rewriting the index rows that name it.
///
/// # Errors
///
/// A string, because every caller of this is already on the `DbResult` rail.
/// The archive being closed, unflushed work still in hand, a block the file
/// cannot produce, and any SQLite failure all surface here; none of them
/// leave a body readable that should have gone, and none of them delete an
/// index row whose block is still the one the archive holds.
pub(super) fn retain_bodies(
    conn: &Connection,
    bodies: &mut BodyArchive,
    cutoff: &str,
) -> Result<RetainOutcome, String> {
    if bodies.has_work() {
        return Err("session body retention needs a flushed archive; blocks are still waiting for index rows".into());
    }
    close_open_block(conn, bodies)?;
    let path = bodies
        .path_in_service()
        .ok_or("session body archive is not open; nothing was retained")?
        .to_path_buf();

    let keep = kept_block_offsets(conn, cutoff)?;
    // The original is still the live archive after this returns.
    let staging = capsem_archive::stage_retained_blocks(&path, &keep).map_err(|error| {
        format!(
            "session body archive {} could not be compacted: {error}",
            path.display()
        )
    })?;
    let moved = staging.map().clone();
    let bytes_reclaimed = staging.bytes_freed();

    let dropped = reindex(conn, cutoff, &moved, &path).map_err(|error| {
        format!(
            "session body retention left the archive {} untouched: its index could not be rewritten: {error}",
            path.display()
        )
    })?;

    commit_staging(conn, bodies, staging, &moved, &path)?;
    // The writer's idea of the file's end is the old file's length, and every
    // block in the new one sits somewhere else.
    bodies.reopen_after_retention();

    Ok(RetainOutcome {
        blocks_dropped: dropped.blocks,
        blocks_kept: moved.len() as u64,
        rows_dropped: dropped.rows,
        bytes_reclaimed,
    })
}

/// Close the open block and commit the rows of its FINAL segment, so the
/// keep query sees it whole and the writer reopened afterwards starts a new
/// block at the compacted file's end.
fn close_open_block(conn: &Connection, bodies: &mut BodyArchive) -> Result<(), String> {
    bodies.close_block();
    let committed = conn.unchecked_transaction().and_then(|tx| {
        bodies.commit_index_rows(&tx)?;
        tx.commit()
    });
    match committed {
        Ok(()) => {
            bodies.index_rows_committed();
            Ok(())
        }
        Err(error) => Err(format!(
            "session body retention could not index the block it closed; nothing was retained: {error}"
        )),
    }
}

/// Put the compacted archive in place, or put the index back.
///
/// A failed rename means the file still holds the old offsets, and the index
/// was committed a moment ago naming the new ones. Reversing the remap is
/// what makes that recoverable rather than merely loud: every surviving body
/// goes back to naming the offset it still occupies.
///
/// The rows of the dropped blocks stay deleted. Their bytes are in the file
/// and now unreferenced, which is the archive's own documented cost for a
/// crash between a block and its index rows -- and those bodies were the ones
/// retention was asked to forget, so forgetting them is not the failure.
fn commit_staging(
    conn: &Connection,
    bodies: &mut BodyArchive,
    staging: capsem_archive::RetainedStaging,
    moved: &BTreeMap<u64, u64>,
    path: &std::path::Path,
) -> Result<(), String> {
    let committed = if take_retention_failure_for_tests(path, RetentionFault::Rename) {
        drop(staging);
        Err("injected rename failure".to_string())
    } else {
        capsem_archive::commit_retained(staging).map_err(|error| error.to_string())
    };
    let Err(error) = committed else {
        return Ok(());
    };
    let restored = if take_retention_failure_for_tests(path, RetentionFault::Restore) {
        Err(rusqlite::Error::InvalidParameterName(
            "injected offset restore failure".to_string(),
        ))
    } else {
        restore_offsets(conn, moved)
    };
    match restored {
        Ok(()) => Err(format!(
            "session body archive {} could not be replaced ({error}); \
             the index was put back and every surviving body still reads",
            path.display()
        )),
        Err(restore_error) => {
            // The index names offsets the file does not have and nothing here
            // can reach them again. Appending more bodies would add rows to a
            // ledger whose existing ones already lie; the session stops
            // archiving, and the bodies it was holding are counted as dropped.
            bodies.give_up("retention");
            Err(format!(
                "session body archive {} could not be replaced ({error}) \
                 and the index could not be put back ({restore_error}); \
                 no further bodies will be stored, and body reads will fail \
                 their integrity check rather than answer wrongly",
                path.display()
            ))
        }
    }
}

/// What the deletes removed.
#[derive(Debug)]
struct Dropped {
    blocks: u64,
    rows: u64,
}

/// A block is kept while its newest segment is inside the window, or while
/// any row still inside the window names it: dedup lets a new row point at
/// bytes in an older block, and that row's body must outlive the block's own
/// age. The keep set and the deletes both read this one predicate, so they
/// cannot disagree about which blocks survive.
const KEPT_BLOCK: &str = "sealed_at >= ?1 OR EXISTS (
    SELECT 1 FROM event_body_blobs AS kept
    WHERE kept.block_offset = body_blocks.block_offset AND kept.created_at >= ?1)";

/// `(block_offset, disk_len)` of every block to keep: the offset to find it
/// and the committed extent to copy.
fn kept_block_offsets(conn: &Connection, cutoff: &str) -> Result<Vec<(u64, u64)>, String> {
    let read_error = |error: rusqlite::Error| format!("session body retention could not read body_blocks: {error}");
    let mut statement = conn
        .prepare_cached(&format!(
            "SELECT block_offset, disk_len FROM body_blocks WHERE {KEPT_BLOCK} ORDER BY block_offset"
        ))
        .map_err(read_error)?;
    let rows = statement
        .query_map(params![cutoff], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(read_error)?;
    let mut keep = Vec::new();
    for row in rows {
        let (offset, disk_len) = row.map_err(read_error)?;
        let (Ok(offset), Ok(disk_len)) = (u64::try_from(offset), u64::try_from(disk_len)) else {
            return Err(format!(
                "body_blocks holds a negative extent ({offset}, {disk_len}); the index is corrupt"
            ));
        };
        keep.push((offset, disk_len));
    }
    Ok(keep)
}

/// Delete the aged-out rows and move the survivors to their new offsets, in
/// one transaction.
///
/// The deletes use the same `KEPT_BLOCK` predicate the keep set came from
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
fn reindex(
    conn: &Connection,
    cutoff: &str,
    moved: &BTreeMap<u64, u64>,
    path: &std::path::Path,
) -> rusqlite::Result<Dropped> {
    let tx = conn.unchecked_transaction()?;
    // The index rows reference `body_blocks(block_offset)`. Remapping moves
    // parent and child one pair at a time, so the two are briefly out of step
    // even though the committed state is consistent. Enforcement is on --
    // rusqlite turns it on for every connection it opens, which
    // `the_writer_connection_enforces_foreign_keys` pins -- so without this
    // the very first pair would be refused.
    tx.execute_batch("PRAGMA defer_foreign_keys = ON")?;
    let rows = tx.execute(
        &format!(
            "DELETE FROM event_body_blobs
             WHERE block_offset IN (SELECT block_offset FROM body_blocks WHERE NOT ({KEPT_BLOCK}))"
        ),
        params![cutoff],
    )?;
    let blocks = tx.execute(
        &format!("DELETE FROM body_blocks WHERE NOT ({KEPT_BLOCK})"),
        params![cutoff],
    )?;

    let survivors: i64 = tx.query_row("SELECT COUNT(*) FROM body_blocks", [], |row| row.get(0))?;
    if survivors != moved.len() as i64 {
        // The staged file holds exactly `moved`, so an index that disagrees
        // would name blocks it does not have.
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "retention staged {} blocks but the index holds {survivors}",
            moved.len()
        )));
    }

    for (old, new) in moved {
        move_block(&tx, *old, *new)?;
    }
    if take_retention_failure_for_tests(path, RetentionFault::IndexTransaction) {
        return Err(rusqlite::Error::InvalidParameterName(
            "injected retention index failure".to_string(),
        ));
    }
    tx.commit()?;
    Ok(Dropped {
        blocks: blocks as u64,
        rows: rows as u64,
    })
}

/// Undo the remap after the rename failed, putting every surviving block back
/// at the offset the unreplaced file still holds it at.
///
/// **Descending order of the new offset**, which is the mirror of the forward
/// pass and collision-free for the mirror reason: writing `n_i -> o_i` from
/// the top down, every offset already written is an `o` above this one, and
/// every offset not yet moved is an `n` below `n_i`, which is at most `o_i`.
fn restore_offsets(conn: &Connection, moved: &BTreeMap<u64, u64>) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute_batch("PRAGMA defer_foreign_keys = ON")?;
    for (old, new) in moved.iter().rev() {
        move_block(&tx, *new, *old)?;
    }
    tx.commit()
}

/// Move one block's rows from `from` to `to`, in both tables.
fn move_block(tx: &rusqlite::Transaction<'_>, from: u64, to: u64) -> rusqlite::Result<()> {
    if from == to {
        return Ok(());
    }
    let (from, to) = (from as i64, to as i64);
    tx.execute(
        "UPDATE body_blocks SET block_offset = ?2 WHERE block_offset = ?1",
        params![from, to],
    )?;
    tx.execute(
        "UPDATE event_body_blobs SET block_offset = ?2 WHERE block_offset = ?1",
        params![from, to],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;
