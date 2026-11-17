//! Environment records use the existing writer, staging transaction and archive.
use super::bodies::staging::ENVIRONMENT_REVISION_SOURCE;
use super::*;
use crate::events::{
    EnvironmentRevisionParent, EnvironmentRevisionState, EnvironmentRevisionTransition, HostEventKind,
    PreparedEnvironmentRevision,
};
use rusqlite::OptionalExtension;

pub(super) fn invalid(message: &'static str) -> rusqlite::Error {
    rusqlite::Error::InvalidParameterName(message.into())
}

pub(super) fn uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| {
        id.to_string() == value && (1..=8).contains(&id.get_version_num()) && id.get_variant() == uuid::Variant::RFC4122
    })
}

pub(super) fn actor(value: &str) -> bool {
    matches!(value, "operator" | "system" | "corporate")
        || value.strip_prefix("agent:").is_some_and(uuid)
        || value
            .strip_prefix("vm:")
            .and_then(|value| value.split_once('/'))
            .is_some_and(|(id, generation)| uuid(id) && uuid(generation))
}

struct ExistingRevision {
    environment_id: String,
    previous_revision_id: Option<String>,
    fork_parent: Option<EnvironmentRevisionParent>,
    timestamp_unix_ms: i64,
    actor: String,
    reason: String,
    request_id: Option<String>,
    payload_event_id: String,
    state: String,
    durable: bool,
    replayable: bool,
}

fn existing(conn: &Connection, id: &str) -> rusqlite::Result<Option<ExistingRevision>> {
    let fields = "environment_id,previous_revision_id,fork_parent_environment_id,fork_parent_revision_id,timestamp_unix_ms,actor,reason,request_id,payload_event_id,state,replayable";
    conn.query_row(&format!("SELECT {fields},1 FROM main.environment_revisions WHERE revision_id=?1 UNION ALL SELECT {fields},0 FROM mem.environment_revisions WHERE revision_id=?1"), [id], |row| {
        let parent_environment: Option<String> = row.get(2)?;
        let parent_revision: Option<String> = row.get(3)?;
        let fork_parent = match (parent_environment,parent_revision) {
            (Some(environment_id),Some(revision_id)) => Some(EnvironmentRevisionParent { environment_id, revision_id }),
            (None,None) => None,
            _ => return Err(invalid("incomplete recorded environment fork lineage")),
        };
        Ok(ExistingRevision { environment_id: row.get(0)?, previous_revision_id: row.get(1)?, fork_parent,
            timestamp_unix_ms: row.get(4)?, actor: row.get(5)?, reason: row.get(6)?, request_id: row.get(7)?,
            payload_event_id: row.get(8)?, state: row.get(9)?, replayable: row.get(10)?, durable: row.get(11)? })
    }).optional()
}

fn validate(record: &PreparedEnvironmentRevision) -> rusqlite::Result<()> {
    if !uuid(&record.environment_id)
        || !uuid(&record.revision_id)
        || record.previous_revision_id.as_deref().is_some_and(|id| !uuid(id))
        || record.request_id.as_deref().is_some_and(|id| !uuid(id))
        || record.fork_parent.as_ref().is_some_and(|parent| {
            !uuid(&parent.environment_id)
                || !uuid(&parent.revision_id)
                || parent.environment_id == record.environment_id
        })
        || record.timestamp_unix_ms < 0
        || !actor(&record.actor)
        || record.reason.trim().is_empty()
        || record.reason.len() > capsem_proto::APPROVAL_CONTEXT_BYTES
        || record.reason.contains('\0')
        || record.payload_event_id.len() != 12
        || !record
            .payload_event_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid("invalid environment revision metadata"));
    }
    record
        .specification
        .validate()
        .map_err(|_| invalid("invalid environment revision specification"))
}

fn committed_parent(
    conn: &Connection,
    environment_id: &str,
    revision_id: &str,
    needs_replay: bool,
) -> rusqlite::Result<ExistingRevision> {
    let Some(parent) = existing(conn, revision_id)? else {
        return Err(invalid("environment predecessor is missing"));
    };
    if parent.environment_id != environment_id
        || !parent.durable
        || parent.state != "committed"
        || (needs_replay && !parent.replayable)
    {
        return Err(invalid(
            "environment predecessor is not a durable replayable committed revision",
        ));
    }
    Ok(parent)
}

pub(super) fn prepare_revision(
    conn: &Connection,
    record: &PreparedEnvironmentRevision,
    bodies: &mut BodyArchive,
) -> rusqlite::Result<()> {
    validate(record)?;
    let encoded =
        capsem_proto::framing::encode_record(record.specification.as_ref(), capsem_proto::ENVIRONMENT_MAX_SPEC_BYTES)
            .map_err(|_| invalid("environment snapshot exceeds its encoded byte limit"))?;
    if let Some(old) = existing(conn, &record.revision_id)? {
        if old.environment_id != record.environment_id
            || old.previous_revision_id != record.previous_revision_id
            || old.fork_parent != record.fork_parent
            || old.timestamp_unix_ms != record.timestamp_unix_ms
            || old.actor != record.actor
            || old.reason != record.reason
            || old.request_id != record.request_id
            || old.payload_event_id != record.payload_event_id
            || !old.replayable
        {
            return Err(invalid("environment revision conflicts with its immutable record"));
        }
        if !old.durable {
            return Err(invalid(
                "environment revision preparation has not reached its durability barrier",
            ));
        }
        let stored = bodies.read_required_bytes(conn, &record.payload_event_id, ENVIRONMENT_REVISION_SOURCE)?;
        if stored != encoded {
            return Err(invalid(
                "environment revision snapshot conflicts with its immutable record",
            ));
        }
        return Ok(());
    }
    super::environment_requests::require_revision_request(conn, record)?;
    let inflight: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM main.environment_revisions WHERE environment_id=?1 AND state='committing' UNION ALL SELECT 1 FROM mem.environment_revisions WHERE environment_id=?1 AND state='committing')", [&record.environment_id], |row| row.get(0))?;
    if inflight {
        return Err(invalid("environment already has a revision commit in flight"));
    }
    match &record.previous_revision_id {
        Some(id) => {
            let previous = committed_parent(conn, &record.environment_id, id, false)?;
            if previous.fork_parent != record.fork_parent {
                return Err(invalid("environment fork lineage is immutable"));
            }
        }
        None => {
            let known: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM main.environment_revisions WHERE environment_id=?1 UNION ALL SELECT 1 FROM mem.environment_revisions WHERE environment_id=?1)", [&record.environment_id], |row| row.get(0))?;
            if known {
                return Err(invalid("existing environment requires an explicit predecessor"));
            }
        }
    }
    if record.previous_revision_id.is_none() {
        if let Some(parent) = &record.fork_parent {
            committed_parent(conn, &parent.environment_id, &parent.revision_id, true)?;
        }
    }
    bodies.stage(
        conn,
        EventBodyBlob {
            event_id: &record.payload_event_id,
            event_type: "environment.revision",
            source_table: ENVIRONMENT_REVISION_SOURCE,
            direction: "payload",
            content_type: Some("application/msgpack"),
            body: Some(&encoded),
            original_bytes: None,
            trace_id: None,
            turn_id: None,
        },
    );
    bodies.required_status()?;
    execute_cached(conn, "INSERT INTO mem.environment_revisions(environment_id,revision_id,previous_revision_id,fork_parent_environment_id,fork_parent_revision_id,timestamp_unix_ms,actor,reason,request_id,state,payload_event_id,replayable) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,'committing',?10,1)",
        rusqlite::params![record.environment_id, record.revision_id, record.previous_revision_id,
            record.fork_parent.as_ref().map(|parent| &parent.environment_id), record.fork_parent.as_ref().map(|parent| &parent.revision_id),
            record.timestamp_unix_ms, record.actor, record.reason, record.request_id, record.payload_event_id])?;
    audit(
        conn,
        record,
        None,
        EnvironmentRevisionState::Committing,
        record.timestamp_unix_ms,
        &record.actor,
        false,
    )
}

#[derive(Serialize)]
struct RevisionAudit<'a> {
    environment_id: &'a str,
    revision_id: &'a str,
    previous_state: Option<EnvironmentRevisionState>,
    state: EnvironmentRevisionState,
}

fn audit(
    conn: &Connection,
    record: &PreparedEnvironmentRevision,
    previous: Option<EnvironmentRevisionState>,
    state: EnvironmentRevisionState,
    at: i64,
    actor: &str,
    durable: bool,
) -> rusqlite::Result<()> {
    audit_identity(
        conn,
        RevisionAudit {
            environment_id: &record.environment_id,
            revision_id: &record.revision_id,
            previous_state: previous,
            state,
        },
        at,
        actor,
        durable,
    )
}

fn audit_identity(
    conn: &Connection,
    detail: RevisionAudit<'_>,
    at: i64,
    actor: &str,
    durable: bool,
) -> rusqlite::Result<()> {
    let state = detail.state;
    let detail = capsem_proto::framing::encode_record(&detail, capsem_proto::APPROVAL_CONTEXT_BYTES)
        .map_err(|_| invalid("environment revision audit exceeds its byte limit"))?;
    let kind = match state {
        EnvironmentRevisionState::Committing => HostEventKind::EnvironmentRevisionPrepared,
        EnvironmentRevisionState::Committed => HostEventKind::EnvironmentRevisionCommitted,
        EnvironmentRevisionState::RecoveredUnapplied => HostEventKind::EnvironmentRevisionRecovered,
    };
    let event = HostEvent {
        timestamp_unix_ms: at,
        kind,
        session_id: None,
        actor: actor.into(),
        detail,
        trace_id: None,
    };
    if durable {
        event_rows::insert_durable_host_event(conn, &event)?;
    } else {
        event_rows::insert_host_event(conn, &event, WriteTarget::Memory)?;
    }
    Ok(())
}

pub(super) fn transition_revision(conn: &Connection, record: &EnvironmentRevisionTransition) -> rusqlite::Result<()> {
    if !uuid(&record.environment_id)
        || !uuid(&record.revision_id)
        || !actor(&record.actor)
        || record.timestamp_unix_ms < 0
    {
        return Err(invalid("invalid environment revision transition metadata"));
    }
    let old = existing(conn, &record.revision_id)?.ok_or_else(|| invalid("environment revision is missing"))?;
    if !old.durable || !old.replayable || old.environment_id != record.environment_id {
        return Err(invalid("environment revision is not durably prepared for transition"));
    }
    let previous =
        EnvironmentRevisionState::parse_str(&old.state).ok_or_else(|| invalid("unknown environment revision state"))?;
    if previous == record.state {
        return Ok(());
    }
    if !matches!(
        (previous, record.state),
        (
            EnvironmentRevisionState::Committing,
            EnvironmentRevisionState::Committed | EnvironmentRevisionState::RecoveredUnapplied
        ) | (
            EnvironmentRevisionState::RecoveredUnapplied,
            EnvironmentRevisionState::Committing
        )
    ) {
        return Err(invalid("invalid environment revision state transition"));
    }
    execute_cached(
        conn,
        "UPDATE main.environment_revisions SET state=?1 WHERE environment_id=?2 AND revision_id=?3",
        rusqlite::params![record.state.as_str(), record.environment_id, record.revision_id],
    )?;
    audit_identity(
        conn,
        RevisionAudit {
            environment_id: &record.environment_id,
            revision_id: &record.revision_id,
            previous_state: Some(previous),
            state: record.state,
        },
        record.timestamp_unix_ms,
        &record.actor,
        true,
    )
}

pub(super) fn committed_reference(
    conn: &Connection,
    reference: &EnvironmentRevisionParent,
    needs_replay: bool,
) -> rusqlite::Result<(Option<EnvironmentRevisionParent>, Option<String>)> {
    let found = committed_parent(conn, &reference.environment_id, &reference.revision_id, needs_replay)?;
    Ok((found.fork_parent, found.request_id))
}
