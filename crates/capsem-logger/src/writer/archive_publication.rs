//! One SQLite publication root for archive generation maintenance.
use super::bodies::BodyArchive;
use super::retention_faults::{take_retention_failure_for_tests, RetentionFault};
use capsem_archive::{BodyLogWriter, GenerationId};
use capsem_foundation::unix::contained::ContainedDir;
use rusqlite::{params, Connection, Transaction};
use std::ffi::OsStr;
use tracing::warn;

#[derive(Debug)]
pub(super) enum PublicationOutcome<T> {
    NotPublished {
        phase: &'static str,
        cause: String,
        orphan_cleanup_pending: bool,
    },
    Published {
        outcome: T,
        old_generation: GenerationId,
        new_generation: GenerationId,
        gc_pending: bool,
    },
    OutcomeUnknown {
        phase: &'static str,
        cause: String,
    },
}

impl<T> PublicationOutcome<T> {
    pub(super) fn into_result(self) -> Result<T, String> {
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

#[derive(Debug)]
pub(super) enum PublishError {
    BeforeCommit(String),
    CommitUnknown(String),
}

pub(super) fn publish_transaction<T>(
    conn: &Connection,
    path: &std::path::Path,
    edit: impl FnOnce(&Transaction<'_>) -> rusqlite::Result<T>,
) -> Result<T, PublishError> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| PublishError::BeforeCommit(error.to_string()))?;
    let value = edit(&tx).map_err(|error| PublishError::BeforeCommit(error.to_string()))?;
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
    Ok(value)
}

pub(super) fn publish_root(
    tx: &Transaction<'_>,
    old: crate::schema::ArchiveState,
    generation_id: GenerationId,
    new_end: u64,
    path: &std::path::Path,
) -> rusqlite::Result<()> {
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
    Ok(())
}

pub(super) fn cleanup_unpublished<T>(
    directory: &ContainedDir,
    candidate: BodyLogWriter,
    generation_id: GenerationId,
    phase: &'static str,
    cause: String,
) -> PublicationOutcome<T> {
    candidate.abandon();
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

pub(super) fn close_archive_block(conn: &Connection, bodies: &mut BodyArchive) -> Result<(), String> {
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

pub(super) fn sync_candidate(directory: &ContainedDir, mut candidate: BodyLogWriter) -> Result<BodyLogWriter, String> {
    let generation_id = candidate.header().generation_id;
    if take_retention_failure_for_tests(directory.path(), RetentionFault::CandidateSync) {
        return cleanup_unpublished(
            directory,
            candidate,
            generation_id,
            "candidate-sync",
            "injected retained-generation sync failure".into(),
        )
        .into_result();
    }
    if let Err(error) = candidate.sync() {
        return cleanup_unpublished(directory, candidate, generation_id, "candidate-sync", error.to_string())
            .into_result();
    }
    if let Err(error) = directory.sync() {
        return cleanup_unpublished(directory, candidate, generation_id, "directory-sync", error.to_string())
            .into_result();
    }

    Ok(candidate)
}

#[cfg(test)]
mod tests;
