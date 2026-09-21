//! Transactional archive-generation retention.
//!
//! Retention copies committed survivor extents into a unique generation and
//! makes that candidate durable before one FULL SQLite transaction remaps the
//! index and switches `archive_state`. The old generation is never rewritten
//! or replaced. SQLite is the sole publication root; deletion is retryable GC.

use std::ffi::OsStr;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::time::{Duration, Instant};

use capsem_archive::{format, BodyLogWriter, FileHeader, GenerationId, FILE_HEADER_BYTES};
use capsem_foundation::unix::contained::{ContainedDir, ContainedOpenOptions};
use capsem_foundation::unix::lock::{self, LockMode};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use tracing::warn;

use super::bodies::{archive_lock_path_for_db, BodyArchive};
use super::retention_faults::{take_retention_failure_for_tests, RetentionFault};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RetainOutcome {
    pub blocks_dropped: u64,
    pub blocks_kept: u64,
    pub rows_dropped: u64,
    /// Logical bytes removed. An old inode may remain pinned by a reader.
    pub bytes_reclaimed: u64,
}

#[derive(Debug)]
enum PublicationOutcome {
    NotPublished {
        phase: &'static str,
        cause: String,
        orphan_cleanup_pending: bool,
    },
    Published {
        outcome: RetainOutcome,
        old_generation: GenerationId,
        new_generation: GenerationId,
        gc_pending: bool,
    },
    OutcomeUnknown {
        phase: &'static str,
        cause: String,
    },
}

impl PublicationOutcome {
    fn into_result(self) -> Result<RetainOutcome, String> {
        match self {
            Self::NotPublished {
                phase,
                cause,
                orphan_cleanup_pending,
            } => Err(format!(
                "archive generation was not published during {phase}: {cause}; orphan cleanup pending: {orphan_cleanup_pending}"
            )),
            Self::Published {
                outcome,
                old_generation,
                new_generation,
                gc_pending,
            } => {
                if gc_pending {
                    warn!(
                        ?old_generation,
                        ?new_generation,
                        "archive generation published; old-generation GC remains pending"
                    );
                }
                Ok(outcome)
            }
            Self::OutcomeUnknown { phase, cause } => Err(format!(
                "archive publication outcome is unknown during {phase}: {cause}; archiving is unavailable until recovery"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct RetentionPlan {
    blocks_kept: u64,
    kept_bytes: u64,
}

const KEPT_BLOCK: &str = "sealed_at >= ?1 OR EXISTS (
    SELECT 1 FROM event_body_blobs AS kept
    WHERE kept.block_offset = body_blocks.block_offset AND kept.created_at >= ?1)";

pub(super) fn retain_bodies(
    conn: &Connection,
    bodies: &mut BodyArchive,
    cutoff: &str,
) -> Result<RetainOutcome, String> {
    if bodies.has_work() {
        return Err("session body retention needs a flushed archive; blocks are still waiting for index rows".into());
    }
    close_open_block(conn, bodies)?;
    let directory = bodies
        .generation_directory()
        .ok_or("session body archive is not open; nothing was retained")?
        .try_clone()
        .map_err(|error| format!("clone archive directory: {error}"))?;
    let db_path = bodies
        .db_path()
        .ok_or("session body archive has no database path")?
        .to_path_buf();
    let state = crate::schema::archive_state(conn).map_err(|error| format!("read archive_state: {error}"))?;
    let mut source = open_source(&directory, state.header, state.committed_end)?;
    let plan = retention_plan(conn, cutoff)?;
    let dense_end = (FILE_HEADER_BYTES as u64)
        .checked_add(plan.kept_bytes)
        .ok_or("retained archive extent overflows u64")?;
    if dense_end == state.committed_end && source.metadata().map_err(io_string)?.len() == dense_end {
        return Ok(RetainOutcome::default());
    }

    let generation_id = GenerationId::new_v4();
    let mut candidate = BodyLogWriter::create_generation(&directory, state.header.archive_id, generation_id)
        .map_err(|error| format!("create retained generation: {error}"))?;
    let copied = match copy_survivors(conn, cutoff, &mut source, &mut candidate) {
        Ok(copied) => copied,
        Err(cause) => {
            return cleanup_unpublished(&directory, candidate, generation_id, "copy", cause).into_result();
        }
    };
    if copied != plan.blocks_kept || candidate.end() != dense_end {
        let candidate_end = candidate.end();
        return cleanup_unpublished(
            &directory,
            candidate,
            generation_id,
            "copy-count-check",
            format!(
                "copied {copied} blocks to {candidate_end}, expected {} blocks ending at {dense_end}",
                plan.blocks_kept
            ),
        )
        .into_result();
    }
    if take_retention_failure_for_tests(directory.path(), RetentionFault::CandidateSync) {
        return cleanup_unpublished(
            &directory,
            candidate,
            generation_id,
            "candidate-sync",
            "injected retained-generation sync failure".into(),
        )
        .into_result();
    }
    if let Err(error) = candidate.sync() {
        return cleanup_unpublished(
            &directory,
            candidate,
            generation_id,
            "candidate-sync",
            error.to_string(),
        )
        .into_result();
    }
    if let Err(error) = directory.sync() {
        return cleanup_unpublished(
            &directory,
            candidate,
            generation_id,
            "directory-sync",
            error.to_string(),
        )
        .into_result();
    }

    let archive_lock = lock::acquire_existing_until(
        &archive_lock_path_for_db(&db_path),
        LockMode::Exclusive,
        Instant::now() + Duration::from_secs(5),
    )
    .map_err(|error| format!("acquire archive publication lock: {error}"))?;
    let current = crate::schema::archive_state(conn).map_err(|error| format!("re-read archive_state: {error}"))?;
    if current != state {
        drop(archive_lock);
        return cleanup_unpublished(
            &directory,
            candidate,
            generation_id,
            "publication-precondition",
            "archive_state changed while the sole writer staged retention".into(),
        )
        .into_result();
    }

    let published = publish_index(
        conn,
        cutoff,
        state,
        generation_id,
        candidate.end(),
        copied,
        directory.path(),
    );
    let (blocks_dropped, rows_dropped) = match published {
        Ok(counts) => counts,
        Err(PublishError::BeforeCommit(cause)) => {
            drop(archive_lock);
            return cleanup_unpublished(&directory, candidate, generation_id, "sqlite-transaction", cause)
                .into_result();
        }
        Err(PublishError::CommitUnknown(cause)) => {
            bodies.give_up("retention_commit_unknown");
            drop(archive_lock);
            drop(candidate);
            return PublicationOutcome::OutcomeUnknown {
                phase: "sqlite-commit",
                cause,
            }
            .into_result();
        }
    };

    bodies.adopt_generation(candidate);
    let old_name = state.header.generation_id.file_name();
    let gc_pending = directory
        .remove_private_file(OsStr::new(&old_name))
        .and_then(|()| directory.sync())
        .is_err();
    drop(archive_lock);
    PublicationOutcome::Published {
        outcome: RetainOutcome {
            blocks_dropped,
            blocks_kept: copied,
            rows_dropped,
            bytes_reclaimed: state.committed_end.saturating_sub(dense_end),
        },
        old_generation: state.header.generation_id,
        new_generation: generation_id,
        gc_pending,
    }
    .into_result()
}

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

fn open_source(directory: &ContainedDir, expected: FileHeader, committed_end: u64) -> Result<File, String> {
    let name = expected.generation_id.file_name();
    let mut source = directory
        .open_file(OsStr::new(&name), ContainedOpenOptions::read_only())
        .map_err(io_string)?;
    if source.metadata().map_err(io_string)?.len() < committed_end {
        return Err("active generation is shorter than archive_state.committed_end".into());
    }
    source.seek(SeekFrom::Start(0)).map_err(io_string)?;
    let mut header = [0u8; FILE_HEADER_BYTES];
    source.read_exact(&mut header).map_err(io_string)?;
    format::decode_file_header(&header)
        .and_then(|header| header.validate(expected.archive_id, expected.generation_id, &name))
        .map_err(|error| error.to_string())?;
    Ok(source)
}

fn retention_plan(conn: &Connection, cutoff: &str) -> Result<RetentionPlan, String> {
    let (blocks_kept, kept_bytes): (i64, i64) = conn
        .query_row(
            &format!(
                "SELECT COUNT(*), COALESCE(SUM(disk_len), 0)
                 FROM body_blocks WHERE {KEPT_BLOCK}"
            ),
            [cutoff],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(sql_string)?;
    Ok(RetentionPlan {
        blocks_kept: u64::try_from(blocks_kept).map_err(|_| "negative kept block count")?,
        kept_bytes: u64::try_from(kept_bytes).map_err(|_| "negative retained byte count")?,
    })
}

fn copy_survivors(
    conn: &Connection,
    cutoff: &str,
    source: &mut File,
    candidate: &mut BodyLogWriter,
) -> Result<u64, String> {
    let mut statement = conn
        .prepare_cached(&format!(
            "SELECT block_offset, disk_len FROM body_blocks
             WHERE {KEPT_BLOCK} ORDER BY block_offset"
        ))
        .map_err(sql_string)?;
    let mut rows = statement.query([cutoff]).map_err(sql_string)?;
    let mut copied = 0u64;
    while let Some(row) = rows.next().map_err(sql_string)? {
        let old: i64 = row.get(0).map_err(sql_string)?;
        let disk_len: i64 = row.get(1).map_err(sql_string)?;
        let old = u64::try_from(old).map_err(|_| "negative block offset".to_string())?;
        let disk_len = u64::try_from(disk_len).map_err(|_| "negative block extent".to_string())?;
        candidate
            .copy_block_from(source, old, disk_len)
            .map_err(|error| error.to_string())?;
        copied = copied.checked_add(1).ok_or("retained block count overflow")?;
    }
    Ok(copied)
}

#[derive(Debug)]
enum PublishError {
    BeforeCommit(String),
    CommitUnknown(String),
}

fn publish_index(
    conn: &Connection,
    cutoff: &str,
    old: crate::schema::ArchiveState,
    generation_id: GenerationId,
    new_end: u64,
    expected_blocks: u64,
    path: &std::path::Path,
) -> Result<(u64, u64), PublishError> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| PublishError::BeforeCommit(error.to_string()))?;
    let counts = publish_inside_transaction(&tx, cutoff, old, generation_id, new_end, expected_blocks, path)
        .map_err(|error| PublishError::BeforeCommit(error.to_string()))?;
    if take_retention_failure_for_tests(path, RetentionFault::CommitUnknownBefore) {
        drop(tx);
        return Err(PublishError::CommitUnknown(
            "injected uncertain COMMIT with G authoritative".into(),
        ));
    }
    tx.commit()
        .map_err(|error| PublishError::CommitUnknown(error.to_string()))?;
    if take_retention_failure_for_tests(path, RetentionFault::CommitUnknownAfter) {
        return Err(PublishError::CommitUnknown(
            "injected uncertain COMMIT with H authoritative".into(),
        ));
    }
    Ok(counts)
}

fn publish_inside_transaction(
    tx: &Transaction<'_>,
    cutoff: &str,
    old: crate::schema::ArchiveState,
    generation_id: GenerationId,
    new_end: u64,
    expected_blocks: u64,
    path: &std::path::Path,
) -> rusqlite::Result<(u64, u64)> {
    tx.execute_batch("PRAGMA defer_foreign_keys = ON")?;
    let rows = tx.execute(
        &format!(
            "DELETE FROM event_body_blobs
             WHERE block_offset IN (SELECT block_offset FROM body_blocks WHERE NOT ({KEPT_BLOCK}))"
        ),
        [cutoff],
    )?;
    let blocks = tx.execute(&format!("DELETE FROM body_blocks WHERE NOT ({KEPT_BLOCK})"), [cutoff])?;
    let (remapped, remapped_end) = remap_survivors(tx)?;
    if remapped != expected_blocks || remapped_end != new_end {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "candidate copied {expected_blocks} blocks ending at {new_end}, but SQL remapped {remapped} ending at {remapped_end}"
        )));
    }
    let revision = old
        .revision
        .checked_add(1)
        .and_then(|revision| i64::try_from(revision).ok())
        .ok_or_else(|| rusqlite::Error::InvalidParameterName("archive_state revision overflow".into()))?;
    let new_end = i64::try_from(new_end)
        .map_err(|_| rusqlite::Error::InvalidParameterName("generation extent exceeds SQLite INTEGER".into()))?;
    let changed = tx.execute(
        "UPDATE archive_state
         SET generation_id = ?1, committed_end = ?2, revision = ?3
         WHERE singleton = 1 AND archive_id = ?4 AND generation_id = ?5
               AND committed_end = ?6 AND revision = ?7",
        params![
            generation_id.as_bytes().as_slice(),
            new_end,
            revision,
            old.header.archive_id.as_bytes().as_slice(),
            old.header.generation_id.as_bytes().as_slice(),
            old.committed_end as i64,
            old.revision as i64,
        ],
    )?;
    if changed != 1 {
        return Err(rusqlite::Error::InvalidParameterName(
            "archive_state changed before publication".into(),
        ));
    }
    if take_retention_failure_for_tests(path, RetentionFault::IndexTransaction) {
        return Err(rusqlite::Error::InvalidParameterName(
            "injected retention publication failure".into(),
        ));
    }
    Ok((blocks as u64, rows as u64))
}

fn remap_survivors(tx: &Transaction<'_>) -> rusqlite::Result<(u64, u64)> {
    let mut last_old = -1i64;
    let mut new_offset = FILE_HEADER_BYTES as u64;
    let mut count = 0u64;
    loop {
        let next = tx
            .query_row(
                "SELECT block_offset, disk_len FROM body_blocks
                 WHERE block_offset > ?1 ORDER BY block_offset LIMIT 1",
                [last_old],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?;
        let Some((old, disk_len)) = next else {
            break;
        };
        let old_u64 =
            u64::try_from(old).map_err(|_| rusqlite::Error::InvalidParameterName("negative block offset".into()))?;
        let disk_len = u64::try_from(disk_len)
            .map_err(|_| rusqlite::Error::InvalidParameterName("negative block extent".into()))?;
        move_block(tx, old_u64, new_offset)?;
        last_old = old;
        new_offset = new_offset
            .checked_add(disk_len)
            .ok_or_else(|| rusqlite::Error::InvalidParameterName("remapped extent overflow".into()))?;
        count = count
            .checked_add(1)
            .ok_or_else(|| rusqlite::Error::InvalidParameterName("remapped block count overflow".into()))?;
    }
    Ok((count, new_offset))
}

fn move_block(tx: &Transaction<'_>, from: u64, to: u64) -> rusqlite::Result<()> {
    if from == to {
        return Ok(());
    }
    let from = i64::try_from(from)
        .map_err(|_| rusqlite::Error::InvalidParameterName("old block offset exceeds SQLite INTEGER".into()))?;
    let to = i64::try_from(to)
        .map_err(|_| rusqlite::Error::InvalidParameterName("new block offset exceeds SQLite INTEGER".into()))?;
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

fn cleanup_unpublished(
    directory: &ContainedDir,
    candidate: BodyLogWriter,
    generation_id: GenerationId,
    phase: &'static str,
    cause: String,
) -> PublicationOutcome {
    drop(candidate);
    let name = generation_id.file_name();
    let orphan_cleanup_pending = directory
        .remove_private_file(OsStr::new(&name))
        .and_then(|()| directory.sync())
        .is_err();
    PublicationOutcome::NotPublished {
        phase,
        cause,
        orphan_cleanup_pending,
    }
}

fn io_string(error: std::io::Error) -> String {
    error.to_string()
}

fn sql_string(error: rusqlite::Error) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests;
