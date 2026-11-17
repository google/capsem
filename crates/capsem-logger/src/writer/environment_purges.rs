//! Durable target plans contain identities, never a second copy of purged values.
use super::bodies::staging::{ENVIRONMENT_REQUEST_SOURCE, ENVIRONMENT_REVISION_SOURCE};
use super::environment::invalid;
use super::*;
use crate::events::environment::purge::PreparedEnvironmentVariablePurge;
use crate::events::environment::request::EnvironmentRequestPayload;
use crate::events::HostEventKind;
use capsem_config::environment::EnvironmentSpecification;
use rusqlite::OptionalExtension;

fn specification(bytes: &[u8]) -> rusqlite::Result<EnvironmentSpecification> {
    let spec: EnvironmentSpecification =
        rmp_serde::from_slice(bytes).map_err(|_| invalid("invalid archived environment purge source"))?;
    spec.validate()
        .map_err(|_| invalid("invalid archived environment purge specification"))?;
    Ok(spec)
}

pub(super) fn prepare(
    conn: &Connection,
    intent: &PreparedEnvironmentVariablePurge,
    bodies: &BodyArchive,
) -> rusqlite::Result<()> {
    intent.validate().map_err(invalid)?;
    let columns = crate::db::PURGE_COLUMNS;
    let existing = conn
        .query_row(
            &format!("SELECT {columns} FROM main.environment_variable_purges p WHERE p.purge_id=?1"),
            [&intent.purge_id],
            crate::db::decode_purge_row,
        )
        .optional()?;
    if let Some(existing) = existing {
        return if existing.intent == *intent {
            Ok(())
        } else {
            Err(invalid("environment variable purge intent is immutable"))
        };
    }
    let purge_inflight: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM main.environment_variable_purges WHERE state IN ('prepared','rewriting','reclaim_pending'))", [], |row| row.get(0))?;
    if purge_inflight {
        return Err(invalid(
            "previous variable purge must finish reclamation before another plan",
        ));
    }
    let inflight:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM main.environment_revisions WHERE environment_id=?1 AND state='committing' UNION ALL SELECT 1 FROM mem.environment_revisions WHERE environment_id=?1 AND state='committing')",
        [&intent.environment_id],|row|row.get(0))?;
    if inflight {
        return Err(invalid("environment must reconcile unfinished revisions before purge"));
    }
    let staged: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM mem.environment_requests WHERE environment_id=?1)",
        [&intent.environment_id],
        |row| row.get(0),
    )?;
    if staged {
        return Err(invalid("environment purge requires a proposal durability barrier"));
    }
    let missing:bool=conn.query_row("SELECT EXISTS(
        SELECT 1 FROM main.environment_revisions r WHERE r.environment_id=?1 AND r.replayable=1
            AND NOT EXISTS(SELECT 1 FROM main.event_body_blobs b WHERE b.source_table='environment_revisions' AND b.event_id=r.payload_event_id AND b.direction='payload')
        UNION ALL
        SELECT 1 FROM main.environment_requests q WHERE q.replayable=1 AND
            (q.environment_id=?1 OR q.result_environment_id=?1 OR EXISTS(SELECT 1 FROM main.environment_revisions r WHERE r.environment_id=?1 AND r.request_id=q.request_id))
            AND NOT EXISTS(SELECT 1 FROM main.event_body_blobs b WHERE b.source_table='environment_requests' AND b.event_id=q.payload_event_id AND b.direction='payload'))",
        [&intent.environment_id],|row|row.get(0))?;
    if missing {
        return Err(invalid("required history payload is missing from the purge scope"));
    }
    let (source_event,source_state,source_time):(String,String,i64)=conn.query_row(
        "SELECT payload_event_id,state,timestamp_unix_ms FROM main.environment_revisions WHERE environment_id=?1 AND revision_id=?2",
        rusqlite::params![intent.environment_id,intent.source_revision_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?)))?;
    if !matches!(source_state.as_str(), "committed" | "recovered_unapplied") || source_time > intent.timestamp_unix_ms {
        return Err(invalid("invalid retained source revision for purge"));
    }
    let reader = bodies.required_reader(conn)?;
    let source =
        BodyArchive::read_required_bytes_with_reader(conn, &reader, &source_event, ENVIRONMENT_REVISION_SOURCE)?;
    let selected = specification(&source)?
        .variables
        .remove(&intent.variable_name)
        .ok_or_else(|| invalid("selected variable has no retained source value"))?;
    let current_event: Option<(String,String,String)> = conn
        .query_row(
            "SELECT p.environment_id,r.payload_event_id,r.state FROM main.environment_revision_references p LEFT JOIN main.environment_revisions r
         ON r.environment_id=p.environment_id AND r.revision_id=p.revision_id
         WHERE p.owner_kind='current_environment' AND p.owner_id=?1 AND p.owner_generation=''",
            [&intent.environment_id],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
        )
        .optional()?;
    if let Some((environment, event, state)) = current_event {
        if environment != intent.environment_id || state != "committed" {
            return Err(invalid(
                "invalid canonical environment owner or revision state before purge",
            ));
        }
        let bytes = BodyArchive::read_required_bytes_with_reader(conn, &reader, &event, ENVIRONMENT_REVISION_SOURCE)?;
        if specification(&bytes)?.variables.get(&intent.variable_name) == Some(&selected) {
            return Err(invalid(
                "selected variable value must be removed from current configuration before history purge",
            ));
        }
    }
    execute_cached(conn,"INSERT INTO main.environment_variable_purges(purge_id,environment_id,source_revision_id,variable_name,timestamp_unix_ms,actor,state) VALUES(?1,?2,?3,?4,?5,?6,'prepared')",
        rusqlite::params![intent.purge_id,intent.environment_id,intent.source_revision_id,intent.variable_name,intent.timestamp_unix_ms,intent.actor])?;
    // Scan in physical order with one leased decoder; history growth does not
    // create an unbounded Rust projection or repeatedly inflate each block.
    let mut statement=conn.prepare_cached("SELECT r.revision_id,r.payload_event_id FROM main.environment_revisions r
        JOIN main.event_body_blobs b ON b.event_id=r.payload_event_id AND b.source_table='environment_revisions' AND b.direction='payload'
        WHERE r.environment_id=?1 ORDER BY b.block_offset,b.body_offset")?;
    let mut rows = statement.query([&intent.environment_id])?;
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        let event: String = row.get(1)?;
        let bytes = BodyArchive::read_required_bytes_with_reader(conn, &reader, &event, ENVIRONMENT_REVISION_SOURCE)?;
        if specification(&bytes)?.variables.get(&intent.variable_name) == Some(&selected) {
            execute_cached(
                conn,
                "INSERT INTO main.environment_revision_purges(purge_id,revision_id) VALUES(?1,?2)",
                rusqlite::params![intent.purge_id, id],
            )?;
        }
    }
    let mut statement=conn.prepare_cached("SELECT q.request_id,q.payload_event_id FROM main.environment_requests q
        JOIN main.event_body_blobs b ON b.event_id=q.payload_event_id AND b.source_table='environment_requests' AND b.direction='payload'
        WHERE q.environment_id=?1 OR q.result_environment_id=?1 OR EXISTS(SELECT 1 FROM main.environment_revisions r WHERE r.environment_id=?1 AND r.request_id=q.request_id)
        ORDER BY b.block_offset,b.body_offset")?;
    let mut rows = statement.query([&intent.environment_id])?;
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        let event: String = row.get(1)?;
        let bytes = BodyArchive::read_required_bytes_with_reader(conn, &reader, &event, ENVIRONMENT_REQUEST_SOURCE)?;
        let payload: EnvironmentRequestPayload =
            rmp_serde::from_slice(&bytes).map_err(|_| invalid("invalid archived purge proposal"))?;
        let variables = match &payload {
            EnvironmentRequestPayload::Change { specification }
            | EnvironmentRequestPayload::ImageUpdate { specification } => {
                specification
                    .validate()
                    .map_err(|_| invalid("invalid archived purge proposal specification"))?;
                &specification.variables
            }
            _ => continue,
        };
        if variables.get(&intent.variable_name) == Some(&selected) {
            execute_cached(
                conn,
                "INSERT INTO main.environment_request_purges(purge_id,request_id) VALUES(?1,?2)",
                rusqlite::params![intent.purge_id, id],
            )?;
        }
    }
    let detail = capsem_proto::framing::encode_record(intent, capsem_proto::APPROVAL_CONTEXT_BYTES)
        .map_err(|_| invalid("environment purge intent audit exceeds its byte limit"))?;
    event_rows::insert_durable_host_event(
        conn,
        &HostEvent {
            timestamp_unix_ms: intent.timestamp_unix_ms,
            kind: HostEventKind::EnvironmentVariablePurgePrepared,
            session_id: None,
            actor: intent.actor.clone(),
            detail,
            trace_id: None,
        },
    )
}

pub(super) mod rewrite;
