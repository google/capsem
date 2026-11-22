//! Application acknowledgement and retention references publish together.
use super::environment::{actor, committed_reference, invalid, uuid};
use super::*;
use crate::events::environment::application::*;
use crate::events::environment::{EnvironmentSandboxReferenceKind, EnvironmentSandboxReferenceMutation};
use crate::events::{EnvironmentRevisionParent, HostEventKind};
use rusqlite::OptionalExtension;

mod boot;
pub(super) use boot::settle_boot;

fn matches_record(m: &EnvironmentApplicationMutation, old: &EnvironmentApplicationRecord, version: i64) -> bool {
    old.version == version
        && old.request_id == m.request_id
        && old.target == m.target
        && old.timestamp_unix_ms == m.timestamp_unix_ms
        && old.actor == m.actor
        && old.state == m.state
        && old.composition == m.composition
        && old.failure_reason == m.failure_reason
}

pub(super) fn apply(
    conn: &Connection,
    m: &EnvironmentApplicationMutation,
    bodies: &BodyArchive,
) -> rusqlite::Result<()> {
    let reference_valid = |value: &EnvironmentRevisionParent| uuid(&value.environment_id) && uuid(&value.revision_id);
    let reason_valid = |value: &str| {
        !value.trim().is_empty() && value.len() <= capsem_proto::APPROVAL_CONTEXT_BYTES && !value.contains('\0')
    };
    let c = &m.composition;
    if !uuid(&m.request_id)
        || !uuid(&m.target.session_id)
        || !uuid(&m.target.generation)
        || !actor(&m.actor)
        || m.timestamp_unix_ms < 0
        || !reason_valid(&c.corporate_policy_id)
        || ![&c.boot, &c.live_policy, &c.desired].into_iter().all(reference_valid)
        || [
            &m.expected_references.boot,
            &m.expected_references.live_policy,
            &m.expected_references.desired,
        ]
        .into_iter()
        .any(|value| value.as_ref().is_some_and(|value| !reference_valid(value)))
        || (m.state == EnvironmentApplicationState::Failed) != m.failure_reason.is_some()
        || m.failure_reason.as_deref().is_some_and(|value| !reason_valid(value))
        || (c.boot == c.desired && c.live_policy == c.desired && c.pending_restart)
    {
        return Err(invalid("invalid environment application metadata"));
    }
    let version = match m.expected_version {
        None => 1,
        Some(value) if value > 0 => value
            .checked_add(1)
            .ok_or_else(|| invalid("application version overflows SQLite INTEGER"))?,
        _ => return Err(invalid("invalid expected application version")),
    };
    let columns = crate::db::APPLICATION_COLUMNS;
    let old=conn.query_row(&format!("SELECT {columns} FROM main.environment_applications WHERE request_id=?1 AND session_id=?2 AND generation=?3"),
        rusqlite::params![m.request_id,m.target.session_id,m.target.generation],crate::db::decode_application_row).optional()?;
    if let Some(old) = &old {
        // A completed historical retry must never reapply its old references.
        if matches_record(m, old, version) {
            return Ok(());
        }
    }
    if old.as_ref().map(|old| old.version) != m.expected_version {
        return Err(invalid("environment application version compare-and-swap conflict"));
    }
    let transition = match old.as_ref().map(|old| old.state) {
        None => m.state == EnvironmentApplicationState::Pending,
        Some(EnvironmentApplicationState::Pending) => matches!(
            m.state,
            EnvironmentApplicationState::Applying | EnvironmentApplicationState::Failed
        ),
        Some(EnvironmentApplicationState::Applying) => matches!(
            m.state,
            EnvironmentApplicationState::Applying
                | EnvironmentApplicationState::Applied
                | EnvironmentApplicationState::Failed
        ),
        Some(EnvironmentApplicationState::Failed) => m.state == EnvironmentApplicationState::Applying,
        Some(EnvironmentApplicationState::Applied) => false,
    };
    if !transition
        || old
            .as_ref()
            .is_some_and(|old| old.timestamp_unix_ms > m.timestamp_unix_ms)
    {
        return Err(invalid("invalid environment application state transition"));
    }
    let approved_base =
        environment_requests::require_application_target(conn, &m.request_id, &m.target, &c.desired, bodies)?;
    if m.expected_version.is_none()
        && approved_base.is_some_and(|base| m.expected_references.desired.as_ref() != Some(&base))
    {
        return Err(invalid("application target no longer matches its reviewed base"));
    }
    // Workload configuration changes only on a real next start. Pending and
    // failed records cannot manufacture a new acknowledged live composition.
    if m.expected_references.boot.as_ref() != Some(&c.boot)
        || (matches!(
            m.state,
            EnvironmentApplicationState::Pending | EnvironmentApplicationState::Failed
        ) && m.expected_references.live_policy.as_ref() != Some(&c.live_policy))
        || (!matches!(
            m.state,
            EnvironmentApplicationState::Pending | EnvironmentApplicationState::Failed
        ) && m.expected_references.live_policy.as_ref() != Some(&c.live_policy)
            && c.live_policy != c.desired)
        || (m.state == EnvironmentApplicationState::Failed
            && m.expected_references.desired.as_ref() != Some(&c.desired))
    {
        return Err(invalid(
            "application composition changes unacknowledged workload or failed policy state",
        ));
    }
    for revision in [&c.boot, &c.live_policy, &c.desired] {
        committed_reference(conn, revision, true)?;
    }
    #[derive(Serialize)]
    struct Audit<'a> {
        request_id: &'a str,
        version: i64,
        state: EnvironmentApplicationState,
        previous_references: &'a EnvironmentSandboxReferences,
        composition: &'a EnvironmentApplicationComposition,
        failure_reason: Option<&'a str>,
    }
    let detail = capsem_proto::framing::encode_record(
        &Audit {
            request_id: &m.request_id,
            version,
            state: m.state,
            previous_references: &m.expected_references,
            composition: c,
            failure_reason: m.failure_reason.as_deref(),
        },
        // Field bounds remain the approval-context bounds. The complete
        // environment record uses the existing aggregate configuration budget.
        capsem_proto::ENVIRONMENT_MAX_SPEC_BYTES,
    )
    .map_err(|_| invalid("environment application audit exceeds its byte limit"))?;
    for (kind, expected, revision) in [
        (
            EnvironmentSandboxReferenceKind::Boot,
            &m.expected_references.boot,
            &c.boot,
        ),
        (
            EnvironmentSandboxReferenceKind::LivePolicy,
            &m.expected_references.live_policy,
            &c.live_policy,
        ),
        (
            EnvironmentSandboxReferenceKind::Desired,
            &m.expected_references.desired,
            &c.desired,
        ),
    ] {
        environment_references::update_sandbox(
            conn,
            &EnvironmentSandboxReferenceMutation {
                session_id: m.target.session_id.clone(),
                generation: m.target.generation.clone(),
                kind,
                expected: expected.clone(),
                revision: Some(revision.clone()),
                timestamp_unix_ms: m.timestamp_unix_ms,
                actor: m.actor.clone(),
            },
        )?;
    }
    execute_cached(conn,"INSERT INTO main.environment_applications(version,request_id,session_id,generation,timestamp_unix_ms,actor,state,boot_environment_id,boot_revision_id,live_policy_environment_id,live_policy_revision_id,desired_environment_id,desired_revision_id,corporate_policy_id,pending_restart,failure_reason)
        VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)
        ON CONFLICT(request_id,session_id,generation) DO UPDATE SET version=excluded.version,timestamp_unix_ms=excluded.timestamp_unix_ms,actor=excluded.actor,state=excluded.state,
            boot_environment_id=excluded.boot_environment_id,boot_revision_id=excluded.boot_revision_id,
            live_policy_environment_id=excluded.live_policy_environment_id,live_policy_revision_id=excluded.live_policy_revision_id,
            desired_environment_id=excluded.desired_environment_id,desired_revision_id=excluded.desired_revision_id,
            corporate_policy_id=excluded.corporate_policy_id,pending_restart=excluded.pending_restart,failure_reason=excluded.failure_reason",
        rusqlite::params![version,m.request_id,m.target.session_id,m.target.generation,m.timestamp_unix_ms,m.actor,m.state.as_str(),
            c.boot.environment_id,c.boot.revision_id,c.live_policy.environment_id,c.live_policy.revision_id,c.desired.environment_id,c.desired.revision_id,
            c.corporate_policy_id,c.pending_restart,m.failure_reason])?;
    event_rows::insert_durable_host_event(
        conn,
        &HostEvent {
            timestamp_unix_ms: m.timestamp_unix_ms,
            kind: HostEventKind::EnvironmentApplicationChanged,
            session_id: Some(m.target.session_id.clone()),
            actor: m.actor.clone(),
            detail,
            trace_id: None,
        },
    )
}
