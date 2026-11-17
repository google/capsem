use super::*;
use rusqlite::OptionalExtension;

pub(super) fn record(conn: &Connection, id: &str) -> Result<EnvironmentVariablePurgeRecord, String> {
    conn.query_row(
        &format!(
            "SELECT {} FROM main.environment_variable_purges p WHERE p.purge_id=?1",
            crate::db::PURGE_COLUMNS
        ),
        [id],
        crate::db::decode_purge_row,
    )
    .optional()
    .map_err(|_| "invalid durable variable purge status".to_owned())?
    .ok_or_else(|| "variable history purge intent is missing".into())
}

pub(super) fn update(
    conn: &Connection,
    execution: &EnvironmentVariablePurgeExecution,
    state: EnvironmentVariablePurgeState,
    old: Option<GenerationId>,
    new: Option<GenerationId>,
    failure: Option<&str>,
) -> rusqlite::Result<()> {
    let changed=conn.execute("UPDATE main.environment_variable_purges SET state=?1,old_generation_id=?2,new_generation_id=?3,failure_reason=?4 WHERE purge_id=?5",
        rusqlite::params![state.as_str(),old.as_ref().map(|id|id.as_bytes().as_slice()),new.as_ref().map(|id|id.as_bytes().as_slice()),failure,execution.purge_id])?;
    if changed != 1 {
        return Err(super::super::invalid("variable purge status disappeared"));
    }
    #[derive(serde::Serialize)]
    struct Audit<'a> {
        purge_id: &'a str,
        state: EnvironmentVariablePurgeState,
        old_generation_id: Option<&'a [u8; 16]>,
        new_generation_id: Option<&'a [u8; 16]>,
        failure_reason: Option<&'a str>,
    }
    let detail = capsem_proto::framing::encode_record(
        &Audit {
            purge_id: &execution.purge_id,
            state,
            old_generation_id: old.as_ref().map(GenerationId::as_bytes),
            new_generation_id: new.as_ref().map(GenerationId::as_bytes),
            failure_reason: failure,
        },
        capsem_proto::ENVIRONMENT_MAX_SPEC_BYTES,
    )
    .map_err(|_| super::super::invalid("variable purge status audit exceeds its byte budget"))?;
    crate::writer::event_rows::insert_durable_host_event(
        conn,
        &crate::HostEvent {
            timestamp_unix_ms: execution.timestamp_unix_ms,
            kind: crate::HostEventKind::EnvironmentVariablePurgeChanged,
            session_id: None,
            actor: "system".into(),
            detail,
            trace_id: None,
        },
    )
}

pub(super) fn durable_update(
    conn: &Connection,
    execution: &EnvironmentVariablePurgeExecution,
    state: EnvironmentVariablePurgeState,
    old: Option<GenerationId>,
    new: Option<GenerationId>,
    failure: Option<&str>,
) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|_| "cannot begin variable purge status transaction")?;
    update(&tx, execution, state, old, new, failure).map_err(|_| "cannot record variable purge status")?;
    tx.commit()
        .map_err(|_| "variable purge status commit is uncertain".into())
}

pub(super) fn publish_markers(
    tx: &rusqlite::Transaction<'_>,
    execution: &EnvironmentVariablePurgeExecution,
) -> rusqlite::Result<()> {
    tx.execute("UPDATE main.environment_revisions SET replayable=0 WHERE revision_id IN (SELECT revision_id FROM main.environment_revision_purges WHERE purge_id=?1)",[&execution.purge_id])?;
    tx.execute("UPDATE main.environment_requests SET replayable=0 WHERE request_id IN (SELECT request_id FROM main.environment_request_purges WHERE purge_id=?1)",[&execution.purge_id])?;
    loop {
        let id:Option<String>=tx.query_row("SELECT q.request_id FROM main.environment_requests q JOIN main.environment_request_purges m ON m.request_id=q.request_id
            WHERE m.purge_id=?1 AND (q.state='pending' OR (q.state='approved' AND q.result_revision_id IS NULL)) LIMIT 1",[&execution.purge_id],|row|row.get(0)).optional()?;
        let Some(id) = id else {
            break;
        };
        crate::writer::environment_requests::invalidate(
            tx,
            &crate::events::environment::request::EnvironmentRequestInvalidation {
                request_id: id,
                timestamp_unix_ms: execution.timestamp_unix_ms,
                actor: "system".into(),
                kind: crate::events::environment::request::EnvironmentRequestInvalidationKind::Stale,
                reason: "required proposal values were purged from history".into(),
            },
        )?;
    }
    Ok(())
}
