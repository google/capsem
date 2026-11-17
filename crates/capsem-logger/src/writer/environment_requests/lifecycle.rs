//! Terminal request outcomes retain original intent and decision attribution.
use super::*;

pub(in crate::writer) fn invalidate(
    conn: &Connection,
    record: &EnvironmentRequestInvalidation,
) -> rusqlite::Result<()> {
    if !uuid(&record.request_id) || !actor(&record.actor) || !reason(&record.reason) || record.timestamp_unix_ms < 0 {
        return Err(invalid("invalid environment request invalidation metadata"));
    }
    let old = existing(conn, &record.request_id)?.ok_or_else(|| invalid("environment request is missing"))?;
    if !old.durable {
        return Err(invalid("environment request invalidation requires durable intent"));
    }
    let existing: Option<(i64,String,String,String)> = conn.query_row(
        "SELECT timestamp_unix_ms,actor,kind,reason FROM main.environment_request_invalidations WHERE request_id=?1",
        [&record.request_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
    ).optional()?;
    if let Some((at, actor, kind, reason)) = existing {
        if at == record.timestamp_unix_ms
            && actor == record.actor
            && kind == record.kind.as_str()
            && reason == record.reason
            && old.state == kind
        {
            return Ok(());
        }
        return Err(invalid("environment request invalidation is immutable"));
    }
    if record.timestamp_unix_ms < old.timestamp_unix_ms
        || old.result.is_some()
        || !matches!(old.state.as_str(), "pending" | "approved")
    {
        return Err(invalid(
            "environment request cannot be invalidated from its current state",
        ));
    }
    if record.kind == EnvironmentRequestInvalidationKind::Expired
        && (old.state != "pending" || record.timestamp_unix_ms < old.expires_at_unix_ms)
    {
        return Err(invalid("environment request has not expired while awaiting review"));
    }
    let decision_time: Option<i64> = conn
        .query_row(
            "SELECT timestamp_unix_ms FROM main.environment_request_decisions WHERE request_id=?1",
            [&record.request_id],
            |row| row.get(0),
        )
        .optional()?;
    if decision_time.is_some_and(|at| at > record.timestamp_unix_ms) {
        return Err(invalid("environment request invalidation precedes its decision"));
    }
    execute_cached(conn,
        "INSERT INTO main.environment_request_invalidations(request_id,timestamp_unix_ms,actor,kind,reason) VALUES(?1,?2,?3,?4,?5)",
        rusqlite::params![record.request_id,record.timestamp_unix_ms,record.actor,record.kind.as_str(),record.reason])?;
    execute_cached(
        conn,
        "UPDATE main.environment_requests SET state=?1 WHERE request_id=?2",
        rusqlite::params![record.kind.as_str(), record.request_id],
    )?;
    audit(
        conn,
        RequestAudit {
            request_id: &record.request_id,
            environment_id: &old.environment_id,
            state: record.kind.as_str(),
            result: None,
        },
        record.timestamp_unix_ms,
        &record.actor,
        HostEventKind::EnvironmentRequestInvalidated,
        true,
        old.requester.as_ref(),
    )
}
