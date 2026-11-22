//! Durable request intent shares the revision writer and required archive.
use super::bodies::staging::ENVIRONMENT_REQUEST_SOURCE;
use super::environment::{actor, committed_reference, invalid, uuid};
use super::*;
use crate::events::environment::request::*;
use crate::events::{EnvironmentRevisionParent, HostEventKind};
use rusqlite::OptionalExtension;

struct ExistingRequest {
    kind: String,
    environment_id: String,
    base_revision_id: Option<String>,
    requester: Option<EnvironmentRequestTarget>,
    timestamp_unix_ms: i64,
    expires_at_unix_ms: i64,
    actor: String,
    reason: String,
    payload_event_id: String,
    state: String,
    replayable: bool,
    result: Option<EnvironmentRevisionParent>,
    source: Option<EnvironmentRevisionParent>,
    durable: bool,
}

fn existing(conn: &Connection, id: &str) -> rusqlite::Result<Option<ExistingRequest>> {
    let fields = "kind,environment_id,base_revision_id,requester_session_id,requester_generation,timestamp_unix_ms,actor,reason,payload_event_id,state,replayable,result_environment_id,result_revision_id,source_environment_id,source_revision_id,expires_at_unix_ms";
    conn.query_row(&format!("SELECT {fields},1 FROM main.environment_requests WHERE request_id=?1 UNION ALL SELECT {fields},0 FROM mem.environment_requests WHERE request_id=?1"), [id], |row| {
        let requester = match (row.get::<_,Option<String>>(3)?, row.get::<_,Option<String>>(4)?) {
            (Some(session_id),Some(generation)) => Some(EnvironmentRequestTarget { session_id, generation }),
            (None,None) => None, _ => return Err(invalid("incomplete environment requester identity")),
        };
        let result = match (row.get::<_,Option<String>>(11)?, row.get::<_,Option<String>>(12)?) {
            (Some(environment_id),Some(revision_id)) => Some(EnvironmentRevisionParent { environment_id, revision_id }),
            (None,None) => None, _ => return Err(invalid("incomplete environment request result identity")),
        };
        let source = match (row.get::<_,Option<String>>(13)?, row.get::<_,Option<String>>(14)?) {
            (Some(environment_id),Some(revision_id)) => Some(EnvironmentRevisionParent {environment_id,revision_id}),
            (None,None) => None, _ => return Err(invalid("incomplete environment request source identity")),
        };
        Ok(ExistingRequest { kind: row.get(0)?, environment_id: row.get(1)?, base_revision_id: row.get(2)?, requester,
            timestamp_unix_ms: row.get(5)?, actor: row.get(6)?, reason: row.get(7)?, payload_event_id: row.get(8)?,
            state: row.get(9)?, replayable: row.get(10)?, result, source, expires_at_unix_ms: row.get(15)?, durable: row.get(16)? })
    }).optional()
}

fn required_payload(
    conn: &Connection,
    request: &ExistingRequest,
    bodies: &BodyArchive,
) -> rusqlite::Result<EnvironmentRequestPayload> {
    let bytes = bodies.read_required_bytes(conn, &request.payload_event_id, ENVIRONMENT_REQUEST_SOURCE)?;
    let payload: EnvironmentRequestPayload =
        rmp_serde::from_slice(&bytes).map_err(|_| invalid("invalid archived environment request payload"))?;
    if payload.kind() != request.kind {
        return Err(invalid(
            "environment request kind metadata does not match its archived intent",
        ));
    }
    if payload.source_revision() != request.source.as_ref() {
        return Err(invalid(
            "environment request source metadata does not match its archived intent",
        ));
    }
    Ok(payload)
}

fn reason(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= capsem_proto::APPROVAL_CONTEXT_BYTES && !value.contains('\0')
}
fn target(value: &EnvironmentRequestTarget) -> bool {
    uuid(&value.session_id) && uuid(&value.generation)
}
fn reference(value: &EnvironmentRevisionParent) -> bool {
    uuid(&value.environment_id) && uuid(&value.revision_id)
}

fn validate_metadata(record: &PreparedEnvironmentRequest) -> rusqlite::Result<()> {
    if !uuid(&record.request_id)
        || !uuid(&record.environment_id)
        || record.base_revision_id.as_deref().is_some_and(|id| !uuid(id))
        || record.requester.as_ref().is_some_and(|value| !target(value))
        || !actor(&record.actor)
        || !reason(&record.reason)
        || record.timestamp_unix_ms < 0
        || record.expires_at_unix_ms <= record.timestamp_unix_ms
        || record.payload_event_id.len() != 12
        || !record
            .payload_event_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid("invalid environment request metadata"));
    }
    let agent = record.actor.starts_with("agent:") || record.actor.starts_with("vm:");
    if agent && (record.requester.is_none() || record.base_revision_id.is_none()) {
        return Err(invalid(
            "agent environment requests require a requester generation and base revision",
        ));
    }
    if let Some(identity) = record.actor.strip_prefix("vm:") {
        let (session, generation) = identity
            .split_once('/')
            .ok_or_else(|| invalid("invalid VM request actor"))?;
        if !record
            .requester
            .as_ref()
            .is_some_and(|requester| requester.session_id == session && requester.generation == generation)
        {
            return Err(invalid("VM request actor does not match its requester generation"));
        }
    }
    Ok(())
}

fn validate(conn: &Connection, record: &PreparedEnvironmentRequest) -> rusqlite::Result<()> {
    let agent = record.actor.starts_with("agent:") || record.actor.starts_with("vm:");
    if let Some(base) = &record.base_revision_id {
        committed_reference(
            conn,
            &EnvironmentRevisionParent {
                environment_id: record.environment_id.clone(),
                revision_id: base.clone(),
            },
            agent,
        )?;
    } else {
        let known: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM main.environment_revisions WHERE environment_id=?1 UNION ALL SELECT 1 FROM mem.environment_revisions WHERE environment_id=?1)", [&record.environment_id], |row| row.get(0))?;
        if known {
            return Err(invalid(
                "existing environment request requires its explicit base revision",
            ));
        }
    }
    match record.payload.as_ref() {
        EnvironmentRequestPayload::Archive { .. } => {
            if record.requester.is_some() {
                return Err(invalid("archive cannot be delegated"));
            }
            archive_base(
                conn,
                &record.environment_id,
                record.base_revision_id.as_deref(),
                &record.actor,
            )?;
        }
        EnvironmentRequestPayload::Change { specification }
        | EnvironmentRequestPayload::ImageUpdate { specification } => {
            specification
                .validate()
                .map_err(|_| invalid("invalid environment request specification"))?;
            if matches!(record.payload.as_ref(), EnvironmentRequestPayload::ImageUpdate { .. })
                && (record.base_revision_id.is_none() || specification.image.is_none())
            {
                return Err(invalid("environment image update requires a base and image selection"));
            }
        }
        EnvironmentRequestPayload::Promotion { source } => {
            if record.actor != "operator"
                || record.requester.is_some()
                || record.base_revision_id.is_none()
                || !reference(source)
            {
                return Err(invalid("invalid operator environment promotion metadata"));
            }
            let (parent, _) = committed_reference(conn, source, true)?;
            if !parent.is_some_and(|parent| parent.environment_id == record.environment_id) {
                return Err(invalid("environment promotion source is not a fork of its target"));
            }
        }
        EnvironmentRequestPayload::Apply {
            revision,
            targets,
            expected,
        } => {
            // Apply fanout shares the existing compiled approval capacity;
            // service application admission accounts for the actual targets.
            let mut sessions = BTreeSet::new();
            if record.actor != "operator"
                || record.requester.is_some()
                || record.base_revision_id.is_none()
                || !reference(revision)
                || revision.environment_id != record.environment_id
                || targets.is_empty()
                || targets.len() > capsem_proto::APPROVAL_PENDING_TOTAL
                || targets
                    .iter()
                    .any(|value| !target(value) || !sessions.insert(value.session_id.as_str()))
            {
                return Err(invalid("invalid operator environment apply targets"));
            }
            committed_reference(conn, revision, true)?;
            if !expected.is_empty() {
                if expected.len() != sessions.len()
                    || expected.keys().any(|session| !sessions.contains(session.as_str()))
                {
                    return Err(invalid("reviewed environment bases must match the selected targets"));
                }
                for base in expected.values() {
                    committed_reference(conn, base, true)?;
                }
            }
        }
    }
    Ok(())
}

fn archive_base(
    conn: &Connection,
    environment_id: &str,
    revision_id: Option<&str>,
    actor: &str,
) -> rusqlite::Result<EnvironmentRevisionParent> {
    let Some(revision_id) = revision_id.filter(|_| actor == "operator") else {
        return Err(invalid("archive requires an operator and an exact environment head"));
    };
    require_active(conn, environment_id)?;
    super::environment_references::check_current(conn, environment_id, Some(revision_id))?;
    let committing: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM main.environment_revisions WHERE environment_id=?1 AND state='committing' \
         UNION ALL SELECT 1 FROM mem.environment_revisions WHERE environment_id=?1 AND state='committing')",
        [environment_id],
        |row| row.get(0),
    )?;
    if committing {
        return Err(invalid("an environment revision is still being published"));
    }
    Ok(EnvironmentRevisionParent {
        environment_id: environment_id.into(),
        revision_id: revision_id.into(),
    })
}

pub(super) fn require_active(conn: &Connection, environment_id: &str) -> rusqlite::Result<()> {
    let archived: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM main.environment_requests WHERE environment_id=?1 \
         AND kind='archive' AND result_revision_id IS NOT NULL)",
        [environment_id],
        |row| row.get(0),
    )?;
    if archived {
        return Err(invalid("environment is archived"));
    }
    Ok(())
}

pub(super) fn prepare(
    conn: &Connection,
    record: &PreparedEnvironmentRequest,
    bodies: &mut BodyArchive,
) -> rusqlite::Result<()> {
    validate_metadata(record)?;
    let encoded =
        capsem_proto::framing::encode_record(record.payload.as_ref(), capsem_proto::ENVIRONMENT_MAX_SPEC_BYTES)
            .map_err(|_| invalid("environment request payload exceeds its encoded byte limit"))?;
    if let Some(old) = existing(conn, &record.request_id)? {
        if old.kind != record.payload.kind()
            || old.environment_id != record.environment_id
            || old.base_revision_id != record.base_revision_id
            || old.requester != record.requester
            || old.timestamp_unix_ms != record.timestamp_unix_ms
            || old.expires_at_unix_ms != record.expires_at_unix_ms
            || old.actor != record.actor
            || old.reason != record.reason
            || old.payload_event_id != record.payload_event_id
            || old.source.as_ref() != record.payload.source_revision()
            || !old.replayable
        {
            return Err(invalid("environment request conflicts with its immutable proposal"));
        }
        if !old.durable {
            return Err(invalid("environment request has not reached its durability barrier"));
        }
        if bodies.read_required_bytes(conn, &record.payload_event_id, ENVIRONMENT_REQUEST_SOURCE)? != encoded {
            return Err(invalid(
                "environment request payload conflicts with its immutable proposal",
            ));
        }
        return Ok(());
    }
    // Identical retries retain their original proposal even after its result
    // creates the environment; only new proposals revalidate live references.
    validate(conn, record)?;
    let duplicate_payload: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM main.environment_requests WHERE payload_event_id=?1 UNION ALL SELECT 1 FROM mem.environment_requests WHERE payload_event_id=?1)", [&record.payload_event_id], |row| row.get(0))?;
    if duplicate_payload {
        return Err(invalid("environment request payload identity is already in use"));
    }
    let pending: i64 = conn.query_row("SELECT (SELECT COUNT(*) FROM main.environment_requests WHERE state='pending')+(SELECT COUNT(*) FROM mem.environment_requests WHERE state='pending')", [], |row| row.get(0))?;
    if pending >= capsem_proto::APPROVAL_PENDING_TOTAL as i64 {
        return Err(invalid("pending environment request capacity is exhausted"));
    }
    if let Some(requester) = &record.requester {
        let pending: i64 = conn.query_row("SELECT (SELECT COUNT(*) FROM main.environment_requests WHERE state='pending' AND requester_session_id=?1)+(SELECT COUNT(*) FROM mem.environment_requests WHERE state='pending' AND requester_session_id=?1)", [&requester.session_id], |row| row.get(0))?;
        if pending >= capsem_proto::APPROVAL_PENDING_PER_SESSION as i64 {
            return Err(invalid("requester environment request capacity is exhausted"));
        }
    }
    bodies.stage(
        conn,
        EventBodyBlob {
            event_id: &record.payload_event_id,
            event_type: "environment.request",
            source_table: ENVIRONMENT_REQUEST_SOURCE,
            direction: "payload",
            content_type: Some("application/msgpack"),
            body: Some(&encoded),
            original_bytes: None,
            trace_id: None,
            turn_id: None,
        },
    );
    bodies.required_status()?;
    execute_cached(conn, "INSERT INTO mem.environment_requests(request_id,kind,environment_id,base_revision_id,requester_session_id,requester_generation,timestamp_unix_ms,actor,reason,state,payload_event_id,replayable,source_environment_id,source_revision_id,expires_at_unix_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,'pending',?10,1,?11,?12,?13)",
        rusqlite::params![record.request_id, record.payload.kind(), record.environment_id, record.base_revision_id,
            record.requester.as_ref().map(|value| &value.session_id), record.requester.as_ref().map(|value| &value.generation),
            record.timestamp_unix_ms, record.actor, record.reason, record.payload_event_id,
            record.payload.source_revision().map(|source| &source.environment_id), record.payload.source_revision().map(|source| &source.revision_id), record.expires_at_unix_ms])?;
    audit(
        conn,
        RequestAudit {
            request_id: &record.request_id,
            environment_id: &record.environment_id,
            state: "pending",
            result: None,
        },
        record.timestamp_unix_ms,
        &record.actor,
        HostEventKind::EnvironmentRequestPrepared,
        false,
        record.requester.as_ref(),
    )
}

pub(super) fn decide(conn: &Connection, record: &EnvironmentRequestDecision) -> rusqlite::Result<()> {
    if !uuid(&record.request_id) || !actor(&record.actor) || !reason(&record.reason) || record.timestamp_unix_ms < 0 {
        return Err(invalid("invalid environment request decision metadata"));
    }
    let old = existing(conn, &record.request_id)?.ok_or_else(|| invalid("environment request is missing"))?;
    if !record.decision.permits_actor(&record.actor, &old.actor) {
        return Err(invalid("environment request decision requires its authorized actor"));
    }
    if !old.durable {
        return Err(invalid("environment request is not durably prepared for decision"));
    }
    let decision: Option<(i64,String,String,String)> = conn.query_row("SELECT timestamp_unix_ms,actor,decision,reason FROM main.environment_request_decisions WHERE request_id=?1", [&record.request_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).optional()?;
    if let Some((at, actor, decision, reason)) = decision {
        if at == record.timestamp_unix_ms
            && actor == record.actor
            && decision == record.decision.as_str()
            && reason == record.reason
            && old.state == record.decision.state()
        {
            return Ok(());
        }
        return Err(invalid("environment request decision is immutable"));
    }
    if record.timestamp_unix_ms < old.timestamp_unix_ms || record.timestamp_unix_ms >= old.expires_at_unix_ms {
        return Err(invalid("environment request decision is outside its review lifetime"));
    }
    if old.state != "pending" || (record.decision == EnvironmentRequestDecisionKind::Approve && !old.replayable) {
        return Err(invalid("environment request cannot be resolved from its current state"));
    }
    let archive = if old.kind == EnvironmentRequestKind::Archive.as_str()
        && record.decision == EnvironmentRequestDecisionKind::Approve
    {
        if old.requester.is_some() {
            return Err(invalid("archive cannot be delegated"));
        }
        Some(archive_base(
            conn,
            &old.environment_id,
            old.base_revision_id.as_deref(),
            &record.actor,
        )?)
    } else {
        None
    };
    execute_cached(conn, "INSERT INTO main.environment_request_decisions(request_id,timestamp_unix_ms,actor,decision,reason) VALUES(?1,?2,?3,?4,?5)", rusqlite::params![record.request_id,record.timestamp_unix_ms,record.actor,record.decision.as_str(),record.reason])?;
    execute_cached(
        conn,
        "UPDATE main.environment_requests SET state=?1 WHERE request_id=?2",
        rusqlite::params![record.decision.state(), record.request_id],
    )?;
    audit(
        conn,
        RequestAudit {
            request_id: &record.request_id,
            environment_id: &old.environment_id,
            state: record.decision.state(),
            result: None,
        },
        record.timestamp_unix_ms,
        &record.actor,
        HostEventKind::EnvironmentRequestDecided,
        true,
        old.requester.as_ref(),
    )?;
    // Approval and archival become visible in the same durable write.
    if let Some(result) = archive {
        store_result(
            conn,
            &record.request_id,
            &old,
            &result,
            record.timestamp_unix_ms,
            &record.actor,
        )?;
    }
    Ok(())
}

pub(super) fn require_approved(conn: &Connection, id: &str) -> rusqlite::Result<()> {
    let old = existing(conn, id)?.ok_or_else(|| invalid("environment request is missing"))?;
    let approved: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM main.environment_request_decisions WHERE request_id=?1 AND decision='approve')",
        [id],
        |row| row.get(0),
    )?;
    if !old.durable || !old.replayable || old.state != "approved" || !approved {
        return Err(invalid("environment request is not durably approved and replayable"));
    }
    Ok(())
}

/// Storage correlation only: the service still verifies caller authority and
/// revalidates the exact archived resolved content before publishing a document.
pub(super) fn require_revision_request(
    conn: &Connection,
    record: &crate::events::PreparedEnvironmentRevision,
    bodies: &BodyArchive,
) -> rusqlite::Result<()> {
    let Some(id) = &record.request_id else {
        return Ok(());
    };
    require_approved(conn, id)?;
    let old = existing(conn, id)?.ok_or_else(|| invalid("environment request is missing"))?;
    if old.result.is_some() || old.kind == "apply" {
        return Err(invalid("environment request cannot prepare another revision"));
    }
    let used: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM main.environment_revisions WHERE request_id=?1 UNION ALL SELECT 1 FROM mem.environment_revisions WHERE request_id=?1)", [id], |row| row.get(0))?;
    if used {
        return Err(invalid("environment request already owns a prepared revision"));
    }
    if old.actor.starts_with("agent:") || old.actor.starts_with("vm:") {
        if record.environment_id == old.environment_id
            || record.previous_revision_id.is_some()
            || !record.fork_parent.as_ref().is_some_and(|parent| {
                parent.environment_id == old.environment_id
                    && Some(&parent.revision_id) == old.base_revision_id.as_ref()
            })
        {
            return Err(invalid(
                "agent request revision must create a fork of its exact approved base",
            ));
        }
    } else if record.environment_id != old.environment_id || record.previous_revision_id != old.base_revision_id {
        return Err(invalid(
            "environment revision does not match its approved target and base",
        ));
    }
    // Approval freezes complete canonical content, including every host pin.
    // Typed archive decoding is owned here; no route/cache can substitute a
    // refreshed selector or a different specification under the same request.
    let payload = required_payload(conn, &old, bodies)?;
    let matches = match payload {
        EnvironmentRequestPayload::Change { specification }
        | EnvironmentRequestPayload::ImageUpdate { specification } => specification == record.specification,
        EnvironmentRequestPayload::Promotion { source } => {
            super::environment::committed_specification(conn, &source, bodies)? == *record.specification
        }
        EnvironmentRequestPayload::Archive { .. } | EnvironmentRequestPayload::Apply { .. } => false,
    };
    if !matches {
        return Err(invalid(
            "environment revision content differs from its approved request",
        ));
    }
    Ok(())
}

pub(super) fn bind_result(
    conn: &Connection,
    record: &EnvironmentRequestResult,
    bodies: &BodyArchive,
) -> rusqlite::Result<()> {
    if !uuid(&record.request_id)
        || !reference(&record.result)
        || record.actor != "operator"
        || record.timestamp_unix_ms < 0
    {
        return Err(invalid("invalid environment request result metadata"));
    }
    require_approved(conn, &record.request_id)?;
    let old = existing(conn, &record.request_id)?.ok_or_else(|| invalid("environment request is missing"))?;
    if let Some(result) = &old.result {
        if result == &record.result {
            return Ok(());
        }
        return Err(invalid("environment request result is immutable"));
    }
    let payload = required_payload(conn, &old, bodies)?;
    let (parent, request_id) = committed_reference(conn, &record.result, true)?;
    match payload {
        EnvironmentRequestPayload::Archive { .. } => {
            return Err(invalid("archive result must be recorded with its decision"));
        }
        EnvironmentRequestPayload::Apply { revision, .. } if revision == record.result => {}
        EnvironmentRequestPayload::Apply { .. } => {
            return Err(invalid("environment apply result does not match its selected revision"))
        }
        _ => {
            if request_id.as_deref() != Some(record.request_id.as_str()) {
                return Err(invalid(
                    "environment result revision does not belong to its approved request",
                ));
            }
            if old.actor.starts_with("agent:") || old.actor.starts_with("vm:") {
                if !parent.is_some_and(|parent| {
                    parent.environment_id == old.environment_id && Some(parent.revision_id) == old.base_revision_id
                }) {
                    return Err(invalid("agent environment result is not a fork of its approved base"));
                }
            } else if record.result.environment_id != old.environment_id {
                return Err(invalid(
                    "operator environment result does not match its requested environment",
                ));
            }
        }
    }
    store_result(
        conn,
        &record.request_id,
        &old,
        &record.result,
        record.timestamp_unix_ms,
        &record.actor,
    )
}

fn store_result(
    conn: &Connection,
    request_id: &str,
    old: &ExistingRequest,
    result: &EnvironmentRevisionParent,
    timestamp_unix_ms: i64,
    actor: &str,
) -> rusqlite::Result<()> {
    execute_cached(
        conn,
        "UPDATE main.environment_requests SET result_environment_id=?1,result_revision_id=?2 WHERE request_id=?3",
        rusqlite::params![result.environment_id, result.revision_id, request_id],
    )?;
    audit(
        conn,
        RequestAudit {
            request_id,
            environment_id: &old.environment_id,
            state: "approved",
            result: Some(result),
        },
        timestamp_unix_ms,
        actor,
        HostEventKind::EnvironmentRequestResultBound,
        true,
        old.requester.as_ref(),
    )
}

#[derive(Serialize)]
struct RequestAudit<'a> {
    request_id: &'a str,
    environment_id: &'a str,
    state: &'a str,
    result: Option<&'a EnvironmentRevisionParent>,
}
fn audit(
    conn: &Connection,
    detail: RequestAudit<'_>,
    at: i64,
    actor: &str,
    kind: HostEventKind,
    durable: bool,
    requester: Option<&EnvironmentRequestTarget>,
) -> rusqlite::Result<()> {
    let detail = capsem_proto::framing::encode_record(&detail, capsem_proto::APPROVAL_CONTEXT_BYTES)
        .map_err(|_| invalid("environment request audit exceeds its byte limit"))?;
    let event = HostEvent {
        timestamp_unix_ms: at,
        kind,
        session_id: requester.map(|value| value.session_id.clone()),
        actor: actor.into(),
        detail,
        trace_id: None,
    };
    if durable {
        event_rows::insert_durable_host_event(conn, &event)
    } else {
        event_rows::insert_host_event(conn, &event, WriteTarget::Memory)
    }
}

mod lifecycle;
pub(super) use lifecycle::invalidate;

/// Correlate application facts with durable selected targets and result. The
/// service still verifies current caller, policy and runtime acknowledgements.
pub(super) fn require_application_target(
    conn: &Connection,
    request_id: &str,
    target: &EnvironmentRequestTarget,
    desired: &EnvironmentRevisionParent,
    bodies: &BodyArchive,
) -> rusqlite::Result<Option<EnvironmentRevisionParent>> {
    require_approved(conn, request_id)?;
    let old = existing(conn, request_id)?.ok_or_else(|| invalid("environment request is missing"))?;
    if old.result.as_ref() != Some(desired) {
        return Err(invalid(
            "application desired revision is not the approved request result",
        ));
    }
    let payload = required_payload(conn, &old, bodies)?;
    match payload {
        EnvironmentRequestPayload::Apply { targets, expected, .. } if targets.contains(target) => {
            Ok(expected.get(&target.session_id).cloned())
        }
        EnvironmentRequestPayload::Change { .. } | EnvironmentRequestPayload::ImageUpdate { .. }
            if old.requester.as_ref() == Some(target) =>
        {
            Ok(None)
        }
        _ => Err(invalid("application target is not selected by its approved request")),
    }
}
