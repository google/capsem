//! Revision references are facts owned by the machine-ledger transaction.
use super::environment::{actor, committed_reference, invalid, uuid};
use super::*;
use crate::events::environment::EnvironmentSandboxReferenceMutation;
use crate::events::{EnvironmentRevisionParent, HostEventKind};
use rusqlite::OptionalExtension;

fn existing(
    conn: &Connection,
    kind: &str,
    owner: &str,
    generation: &str,
) -> rusqlite::Result<Option<EnvironmentRevisionParent>> {
    conn.query_row("SELECT environment_id,revision_id FROM main.environment_revision_references WHERE owner_kind=?1 AND owner_id=?2 AND owner_generation=?3",
        rusqlite::params![kind,owner,generation], |row| Ok(EnvironmentRevisionParent { environment_id: row.get(0)?, revision_id: row.get(1)? })).optional()
}

/// The reference is a transactionally maintained retention/coherence fact;
/// service-owned TOML and verified recovery still determine what is current.
pub(super) fn check_current(
    conn: &Connection,
    environment: &str,
    expected_revision: Option<&str>,
) -> rusqlite::Result<()> {
    let current = existing(conn, "current_environment", environment, "")?;
    if let Some(current) = &current {
        if current.environment_id != environment {
            return Err(invalid("canonical reference points at another environment"));
        }
        committed_reference(conn, current, false)?;
    }
    if current.as_ref().map(|current| current.revision_id.as_str()) != expected_revision {
        return Err(invalid(
            "canonical environment reference does not match the verified predecessor",
        ));
    }
    Ok(())
}

pub(super) fn commit_current(
    conn: &Connection,
    environment: &str,
    revision: &str,
    predecessor: Option<&str>,
) -> rusqlite::Result<()> {
    check_current(conn, environment, predecessor)?;
    committed_reference(
        conn,
        &EnvironmentRevisionParent {
            environment_id: environment.into(),
            revision_id: revision.into(),
        },
        true,
    )?;
    execute_cached(conn, "INSERT INTO main.environment_revision_references(owner_kind,owner_id,owner_generation,environment_id,revision_id) VALUES('current_environment',?1,'',?1,?2) ON CONFLICT(owner_kind,owner_id,owner_generation) DO UPDATE SET environment_id=excluded.environment_id,revision_id=excluded.revision_id",
        rusqlite::params![environment,revision])?;
    Ok(())
}

pub(super) fn update_sandbox(conn: &Connection, record: &EnvironmentSandboxReferenceMutation) -> rusqlite::Result<()> {
    let valid_reference = |value: &EnvironmentRevisionParent| uuid(&value.environment_id) && uuid(&value.revision_id);
    if !uuid(&record.session_id)
        || !uuid(&record.generation)
        || !actor(&record.actor)
        || record.timestamp_unix_ms < 0
        || record.expected.as_ref().is_some_and(|value| !valid_reference(value))
        || record.revision.as_ref().is_some_and(|value| !valid_reference(value))
    {
        return Err(invalid("invalid sandbox environment reference metadata"));
    }
    if let Some(identity) = record.actor.strip_prefix("vm:") {
        let (session, generation) = identity
            .split_once('/')
            .ok_or_else(|| invalid("invalid VM reference actor"))?;
        if session != record.session_id || generation != record.generation {
            return Err(invalid("VM reference actor does not match its sandbox generation"));
        }
    }
    let kind = record.kind.as_str();
    let previous = existing(conn, kind, &record.session_id, &record.generation)?;
    if previous != record.expected {
        return Err(invalid("sandbox environment reference compare-and-swap conflict"));
    }
    if previous == record.revision {
        return Ok(());
    }
    if let Some(revision) = &record.revision {
        committed_reference(conn, revision, true)?;
        execute_cached(conn,"INSERT INTO main.environment_revision_references(owner_kind,owner_id,owner_generation,environment_id,revision_id) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(owner_kind,owner_id,owner_generation) DO UPDATE SET environment_id=excluded.environment_id,revision_id=excluded.revision_id",
            rusqlite::params![kind,record.session_id,record.generation,revision.environment_id,revision.revision_id])?;
    } else {
        execute_cached(conn,"DELETE FROM main.environment_revision_references WHERE owner_kind=?1 AND owner_id=?2 AND owner_generation=?3",rusqlite::params![kind,record.session_id,record.generation])?;
    }
    #[derive(Serialize)]
    struct ReferenceAudit<'a> {
        kind: &'a str,
        generation: &'a str,
        previous: &'a Option<EnvironmentRevisionParent>,
        revision: &'a Option<EnvironmentRevisionParent>,
    }
    let detail = capsem_proto::framing::encode_record(
        &ReferenceAudit {
            kind,
            generation: &record.generation,
            previous: &previous,
            revision: &record.revision,
        },
        capsem_proto::APPROVAL_CONTEXT_BYTES,
    )
    .map_err(|_| invalid("sandbox reference audit exceeds its byte limit"))?;
    event_rows::insert_durable_host_event(
        conn,
        &HostEvent {
            timestamp_unix_ms: record.timestamp_unix_ms,
            kind: HostEventKind::EnvironmentReferenceChanged,
            session_id: Some(record.session_id.clone()),
            actor: record.actor.clone(),
            detail,
            trace_id: None,
        },
    )
}
