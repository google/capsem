//! Settle deferred application using a verified later boot of the approved target.
use super::*;

pub(in crate::writer) fn settle_boot(conn: &Connection, boot: &EnvironmentApplicationBoot) -> rusqlite::Result<()> {
    let target = &boot.target;
    let revision = &boot.revision;
    if !uuid(&target.session_id)
        || !uuid(&target.generation)
        || !uuid(&revision.environment_id)
        || !uuid(&revision.revision_id)
        || boot.timestamp_unix_ms < 0
    {
        return Err(invalid("invalid environment boot receipt"));
    }
    let waiting: Option<(String, String, i64, i64)> = conn
        .query_row(
            "SELECT a.request_id,a.generation,a.version,a.timestamp_unix_ms
         FROM main.environment_revisions v
         JOIN main.environment_requests r ON r.request_id=v.request_id
         JOIN main.environment_applications a ON a.request_id=r.request_id
            AND a.session_id=r.requester_session_id AND a.generation=r.requester_generation
         WHERE v.environment_id=?1 AND v.revision_id=?2 AND v.state='committed'
            AND r.state='approved' AND r.result_environment_id=?1 AND r.result_revision_id=?2
            AND a.desired_environment_id=?1 AND a.desired_revision_id=?2
            AND a.session_id=?3 AND a.generation<>?4 AND a.state='applied' AND a.pending_restart=1",
            rusqlite::params![
                revision.environment_id,
                revision.revision_id,
                target.session_id,
                target.generation
            ],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    let Some((request_id, prior_generation, version, applied_at)) = waiting else {
        return Ok(());
    };
    let acknowledged: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM main.environment_revision_references
         WHERE owner_kind='sandbox_boot' AND owner_id=?1 AND owner_generation=?2
         AND environment_id=?3 AND revision_id=?4)",
        rusqlite::params![
            target.session_id,
            target.generation,
            revision.environment_id,
            revision.revision_id
        ],
        |row| row.get(0),
    )?;
    if !acknowledged || boot.timestamp_unix_ms < applied_at {
        return Err(invalid(
            "environment boot receipt requires the acknowledged successor generation and exact result",
        ));
    }
    let next = version
        .checked_add(1)
        .ok_or_else(|| invalid("application version overflows SQLite INTEGER"))?;
    let detail = capsem_proto::framing::encode_record(
        &(request_id.as_str(), prior_generation.as_str(), next, boot),
        capsem_proto::ENVIRONMENT_MAX_SPEC_BYTES,
    )
    .map_err(|_| invalid("environment boot receipt exceeds its byte limit"))?;
    execute_cached(
        conn,
        "UPDATE main.environment_applications SET pending_restart=0,version=?1
         WHERE request_id=?2 AND session_id=?3 AND generation=?4",
        rusqlite::params![next, request_id, target.session_id, prior_generation],
    )?;
    event_rows::insert_durable_host_event(
        conn,
        &HostEvent {
            timestamp_unix_ms: boot.timestamp_unix_ms,
            kind: HostEventKind::EnvironmentApplicationChanged,
            session_id: Some(target.session_id.clone()),
            actor: "system".into(),
            detail,
            trace_id: None,
        },
    )
}
