//! Recompress every indexed span before atomically publishing redacted history.
use super::super::archive_publication::{self, PublishError};
use super::super::bodies::BodyArchive;
use crate::events::environment::purge::{
    EnvironmentVariablePurgeExecution, EnvironmentVariablePurgeRecord, EnvironmentVariablePurgeState,
};
use capsem_archive::{BodyLogWriter, GenerationId};
use rusqlite::Connection;

mod payloads;
mod reclaim;
mod status;

impl crate::writer::DbWriter {
    /// Execute a durable purge plan on the owning writer, after queued writes flush.
    pub async fn purge_variable_history(
        &self,
        execution: EnvironmentVariablePurgeExecution,
    ) -> Result<EnvironmentVariablePurgeRecord, String> {
        execution.validate().map_err(str::to_owned)?;
        if self.remote.is_some() {
            return Err("variable history purge requires the ledger maintainer channel".into());
        }
        let tx = self.clone_sender().ok_or("db writer is shut down")?;
        let (reply, rx) = tokio::sync::oneshot::channel();
        crate::writer::send_with_backpressure(&tx, crate::writer::WriterMessage::Purge { execution, reply }).await?;
        rx.await
            .map_err(|_| "writer dropped the variable purge reply".to_owned())?
    }
}

pub(in crate::writer) fn run(
    conn: &Connection,
    bodies: &mut BodyArchive,
    execution: &EnvironmentVariablePurgeExecution,
) -> Result<EnvironmentVariablePurgeRecord, String> {
    execution.validate().map_err(str::to_owned)?;
    let record = status::record(conn, &execution.purge_id)?;
    if execution.timestamp_unix_ms < record.intent.timestamp_unix_ms {
        return Err("variable purge execution predates its intent".into());
    }
    match record.state {
        EnvironmentVariablePurgeState::Committed => return Ok(record),
        EnvironmentVariablePurgeState::ReclaimPending => return reclaim::finish(conn, bodies, execution),
        EnvironmentVariablePurgeState::Failed => return Err("variable purge plan failed; prepare a new plan".into()),
        _ => (),
    }
    if bodies.has_work() {
        return Err("variable purge requires a flushed body archive".into());
    }
    archive_publication::close_archive_block(conn, bodies)?;
    let directory = bodies
        .generation_directory()
        .ok_or("variable purge archive is unavailable")?
        .try_clone()
        .map_err(|_| "cannot clone variable purge archive directory")?;
    let db_path = bodies
        .db_path()
        .ok_or("variable purge ledger is unavailable")?
        .to_path_buf();
    let old = crate::schema::archive_state(conn).map_err(|_| "cannot read variable purge publication root")?;
    let source = Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| "cannot open variable purge source snapshot")?;
    let snapshot = source
        .unchecked_transaction()
        .map_err(|_| "cannot begin variable purge source snapshot")?;
    if crate::schema::archive_state(&snapshot).map_err(|_| "cannot read variable purge source root")? != old {
        return Err("variable purge source root changed".into());
    }
    let reader = bodies
        .required_reader(&snapshot)
        .map_err(|_| "cannot open variable purge source archive")?;
    let selected = payloads::check_current(&snapshot, &reader, &record)?;
    let generation = GenerationId::generate();
    let candidate =
        BodyLogWriter::create_generation_with_codecs(&directory, old.header.archive_id, generation, bodies.codecs())
            .map_err(|_| "cannot create variable purge candidate generation")?;
    let mut candidate = Some(candidate);
    if let Err(error) = status::durable_update(
        conn,
        execution,
        EnvironmentVariablePurgeState::Rewriting,
        Some(old.header.generation_id),
        Some(generation),
        None,
    ) {
        return archive_publication::cleanup_unpublished(
            &directory,
            candidate.take().unwrap(),
            generation,
            "purge-status",
            error,
        )
        .into_result();
    }
    // One writer owns both halves. The separate read transaction keeps old
    // index offsets stable while this transaction replaces them in place.
    let mut stale_plan = false;
    let result = archive_publication::publish_transaction(conn, directory.path(), |tx| {
        tx.execute_batch("PRAGMA defer_foreign_keys=ON; DELETE FROM main.body_blocks;")?;
        payloads::rewrite(tx, &snapshot, &reader, candidate.as_mut().unwrap(), &record, &selected).map_err(
            |cause| {
                stale_plan = cause == payloads::PLAN_CHANGED;
                super::invalid("cannot stage durable purge generation")
            },
        )?;
        let synced = archive_publication::sync_candidate(&directory, candidate.take().unwrap())
            .map_err(|_| super::invalid("cannot stage durable purge generation"))?;
        candidate = Some(synced);
        let _publication = capsem_foundation::unix::lock::acquire_existing_until(
            &crate::writer::archive_lock_path_for_db(&db_path),
            capsem_foundation::unix::lock::LockMode::Exclusive,
            std::time::Instant::now() + std::time::Duration::from_secs(5),
        )
        .map_err(|_| super::invalid("cannot acquire variable purge publication lock"))?;
        if crate::schema::archive_state(tx)? != old {
            return Err(super::invalid("variable purge publication root changed"));
        }
        status::publish_markers(tx, execution)?;
        status::update(
            tx,
            execution,
            EnvironmentVariablePurgeState::ReclaimPending,
            Some(old.header.generation_id),
            Some(generation),
            None,
        )?;
        archive_publication::publish_root(tx, old, generation, candidate.as_ref().unwrap().end(), directory.path())?;
        // Keep the publication lock through COMMIT by returning its guard.
        Ok(_publication)
    });
    match result {
        Ok(publication) => {
            bodies.adopt_generation(candidate.take().unwrap());
            drop(publication);
        }
        Err(PublishError::BeforeCommit(_)) => {
            let error = if stale_plan {
                payloads::PLAN_CHANGED
            } else {
                "variable history rewrite was not published"
            }
            .to_owned();
            if let Some(candidate) = candidate.take() {
                let _: Result<(), String> = archive_publication::cleanup_unpublished(
                    &directory,
                    candidate,
                    generation,
                    "purge-rewrite",
                    error.clone(),
                )
                .into_result();
            }
            status::durable_update(
                conn,
                execution,
                if stale_plan {
                    EnvironmentVariablePurgeState::Failed
                } else {
                    EnvironmentVariablePurgeState::Prepared
                },
                None,
                None,
                Some(&error),
            )?;
            return Err(error);
        }
        Err(PublishError::CommitUnknown(_)) => {
            bodies.give_up("variable_purge_commit_unknown");
            if let Some(candidate) = candidate.take() {
                candidate.abandon();
            }
            return Err("variable history publication outcome is unknown; writer recovery is required".into());
        }
    }
    drop(reader);
    drop(snapshot);
    drop(source);
    reclaim::finish(conn, bodies, execution)
}
