use super::*;
use crate::events::environment::request::EnvironmentRequestPayload;
use capsem_archive::{ArchiveError, BodyLogReader};
use rusqlite::{params, OptionalExtension};

pub(super) const PLAN_CHANGED: &str = "variable purge plan changed; prepare a new plan before retrying";

pub(super) fn check_current(
    conn: &Connection,
    reader: &BodyLogReader,
    record: &EnvironmentVariablePurgeRecord,
) -> Result<String, String> {
    let intent = &record.intent;
    let source: String = conn
        .query_row(
            "SELECT payload_event_id FROM environment_revisions WHERE environment_id=?1 AND revision_id=?2",
            params![intent.environment_id, intent.source_revision_id],
            |row| row.get(0),
        )
        .map_err(|_| "purge source revision is unavailable")?;
    let bytes =
        BodyArchive::read_required_bytes_with_reader(conn, reader, &source, super::super::ENVIRONMENT_REVISION_SOURCE)
            .map_err(|_| "purge source payload is unavailable")?;
    let selected = super::super::specification(&bytes)
        .map_err(|_| "purge source payload is invalid")?
        .variables
        .remove(&intent.variable_name)
        .ok_or("purge source value is unavailable")?;
    let current: Option<(String, String, String)> = conn.query_row("SELECT p.environment_id,r.payload_event_id,r.state FROM environment_revision_references p LEFT JOIN environment_revisions r ON r.environment_id=p.environment_id AND r.revision_id=p.revision_id WHERE p.owner_kind='current_environment' AND p.owner_id=?1 AND p.owner_generation=''", [&intent.environment_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).optional().map_err(|_| "purge current configuration is invalid")?;
    if let Some((environment, event, state)) = current {
        if environment != intent.environment_id || state != "committed" {
            return Err("purge current configuration owner is invalid".into());
        }
        let bytes = BodyArchive::read_required_bytes_with_reader(
            conn,
            reader,
            &event,
            super::super::ENVIRONMENT_REVISION_SOURCE,
        )
        .map_err(|_| "purge current payload is unavailable")?;
        if super::super::specification(&bytes)
            .map_err(|_| "purge current specification is invalid")?
            .variables
            .get(&intent.variable_name)
            == Some(&selected)
        {
            return Err("selected value must be removed from current configuration before history purge".into());
        }
    }
    Ok(selected)
}

pub(super) fn rewrite(
    tx: &rusqlite::Transaction<'_>,
    source: &Connection,
    reader: &BodyLogReader,
    candidate: &mut BodyLogWriter,
    record: &EnvironmentVariablePurgeRecord,
    selected: &str,
) -> Result<(), String> {
    let mut statement = source.prepare(&format!("SELECT {}, blocks.sealed_at,
        CASE WHEN b.direction='payload' AND b.source_table='environment_revisions' THEN EXISTS (
            SELECT 1 FROM environment_revision_purges m JOIN environment_revisions r ON r.revision_id=m.revision_id WHERE m.purge_id=?1 AND r.payload_event_id=b.event_id)
        WHEN b.direction='payload' AND b.source_table='environment_requests' THEN EXISTS (
            SELECT 1 FROM environment_request_purges m JOIN environment_requests q ON q.request_id=m.request_id WHERE m.purge_id=?1 AND q.payload_event_id=b.event_id)
        ELSE 0 END,
        CASE WHEN b.direction='payload' AND b.source_table='environment_revisions' THEN EXISTS (
            SELECT 1 FROM environment_revisions r WHERE r.payload_event_id=b.event_id AND r.environment_id=?2)
        WHEN b.direction='payload' AND b.source_table='environment_requests' THEN EXISTS (
            SELECT 1 FROM environment_requests q WHERE q.payload_event_id=b.event_id AND (q.environment_id=?2 OR q.result_environment_id=?2 OR EXISTS (
                SELECT 1 FROM environment_revisions r WHERE r.request_id=q.request_id AND r.environment_id=?2)))
        ELSE 0 END FROM {} ORDER BY b.block_offset,b.body_offset,b.id", crate::db::ARCHIVE_INDEX_COLUMNS, crate::db::ARCHIVE_INDEX_FROM)).map_err(|_| "cannot stream purge source index")?;
    let mut rows = statement
        .query(params![record.intent.purge_id, record.intent.environment_id])
        .map_err(|_| "cannot query purge source index")?;
    let mut block = None;
    let mut sealed_at = String::new();
    let mut count = 0i64;
    let mut changed = 0i64;
    while let Some(row) = rows.next().map_err(|_| "cannot read purge source index")? {
        let index = crate::db::archive_index_row(row)?;
        let old_block = index.reference.block_offset;
        if block != Some(old_block) {
            close(tx, candidate, &sealed_at)?;
            block = Some(old_block);
            sealed_at = crate::db::archive_sql_text(row, 12, crate::ledger_protocol::MAX_TIMESTAMP_BYTES)?;
        }
        let redact: bool = row.get(13).map_err(|_| "invalid purge target index")?;
        let mut body = crate::db::read_archive_index(reader, index)?;
        let in_scope: bool = row.get(14).map_err(|_| "invalid purge scope index")?;
        if in_scope && matches_value(&body.source_table, &body.bytes, &record.intent.variable_name, selected)? != redact
        {
            return Err(PLAN_CHANGED.into());
        }
        if redact {
            body.bytes = redact_payload(&body.source_table, &body.bytes, &record.intent.variable_name)?;
            body.original_bytes = body.bytes.len() as u64;
            body.truncated = false;
            // The persisted body-index checksum identifies the newly archived
            // span; the old checksum must never describe redacted bytes.
            body.body_hash = crate::content_digest(&body.bytes);
            changed += 1;
        }
        let reference = match candidate.stage(&body.bytes) {
            Err(ArchiveError::BlockFull) => {
                close(tx, candidate, &sealed_at)?;
                candidate.stage(&body.bytes)
            }
            result => result,
        }
        .map_err(|_| "cannot encode purge candidate body")?;
        let updated = tx.execute("UPDATE main.event_body_blobs SET block_offset=?1,body_offset=?2,body_len=?3,stored_bytes=?3,original_bytes=?4,truncated=?5,body_hash=?6 WHERE event_id=?7 AND source_table=?8 AND direction=?9", params![reference.block_offset as i64,reference.offset,reference.len,body.original_bytes as i64,body.truncated,body.body_hash,body.event_id,body.source_table,body.direction.as_str()]).map_err(|_| "cannot remap purge candidate body")?;
        if updated != 1 {
            return Err("purge source index changed".into());
        }
        if candidate.wants_close() {
            close(tx, candidate, &sealed_at)?;
        }
        count += 1;
    }
    close(tx, candidate, &sealed_at)?;
    let expected: i64 = source
        .query_row("SELECT COUNT(*) FROM event_body_blobs", [], |row| row.get(0))
        .map_err(|_| "cannot verify purge source index count")?;
    if count != expected || changed != record.revision_count + record.request_count {
        return Err("purge target or source payload count changed".into());
    }
    Ok(())
}

fn close(tx: &Connection, candidate: &mut BodyLogWriter, sealed_at: &str) -> Result<(), String> {
    if let Some(segment) = candidate
        .close_block()
        .map_err(|_| "cannot close purge compression block")?
    {
        tx.execute(
            "INSERT INTO main.body_blocks(block_offset,raw_len,disk_len,sealed_at) VALUES(?1,?2,?3,?4)",
            params![
                segment.block_offset as i64,
                segment.raw_len,
                segment.disk_len as i64,
                sealed_at
            ],
        )
        .map_err(|_| "cannot index purge compression block")?;
    }
    Ok(())
}

fn redact_payload(source: &str, bytes: &[u8], key: &str) -> Result<Vec<u8>, String> {
    if source == super::super::ENVIRONMENT_REVISION_SOURCE {
        let mut spec = super::super::specification(bytes).map_err(|_| "invalid purge revision payload")?;
        if spec.variables.remove(key).is_none() {
            return Err("planned purge variable is absent".into());
        }
        spec.validate().map_err(|_| "invalid redacted revision payload")?;
        capsem_proto::framing::encode_record(&spec, capsem_proto::ENVIRONMENT_MAX_SPEC_BYTES)
            .map_err(|_| "redacted revision exceeds its byte budget".into())
    } else {
        let mut payload: EnvironmentRequestPayload =
            rmp_serde::from_slice(bytes).map_err(|_| "invalid purge proposal payload")?;
        match &mut payload {
            EnvironmentRequestPayload::Change { specification }
            | EnvironmentRequestPayload::ImageUpdate { specification } => {
                if specification.variables.remove(key).is_none() {
                    return Err("planned proposal variable is absent".into());
                }
                specification
                    .validate()
                    .map_err(|_| "invalid redacted proposal payload")?;
            }
            _ => return Err("planned purge proposal has no variables".into()),
        }
        capsem_proto::framing::encode_record(&payload, capsem_proto::ENVIRONMENT_MAX_SPEC_BYTES)
            .map_err(|_| "redacted proposal exceeds its byte budget".into())
    }
}

fn matches_value(source: &str, bytes: &[u8], key: &str, selected: &str) -> Result<bool, String> {
    if source == super::super::ENVIRONMENT_REVISION_SOURCE {
        let spec = super::super::specification(bytes).map_err(|_| "invalid purge scope revision")?;
        Ok(spec.variables.get(key).is_some_and(|value| value == selected))
    } else {
        let payload: EnvironmentRequestPayload =
            rmp_serde::from_slice(bytes).map_err(|_| "invalid purge scope proposal")?;
        match payload {
            EnvironmentRequestPayload::Change { specification }
            | EnvironmentRequestPayload::ImageUpdate { specification } => {
                specification
                    .validate()
                    .map_err(|_| "invalid purge scope specification")?;
                Ok(specification.variables.get(key).is_some_and(|value| value == selected))
            }
            _ => Ok(false),
        }
    }
}
