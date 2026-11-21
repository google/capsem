use super::*;
use crate::events::environment::application::EnvironmentApplicationState;
use crate::events::environment::purge::EnvironmentVariablePurgeState;
use crate::events::environment::request::{
    EnvironmentRequestDecisionKind, EnvironmentRequestInvalidationKind, EnvironmentRequestKind,
    EnvironmentRequestPayload, EnvironmentRequestState, EnvironmentRequestTarget,
};
use crate::events::{environment::EnvironmentSandboxReferenceKind, EnvironmentRevisionParent};
use std::collections::BTreeSet;

pub(super) fn id(value: &str) -> bool {
    crate::db::valid_environment_id(value)
}
fn text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.contains('\0')
}
fn actor(value: &str) -> bool {
    text(value, 2 * uuid::fmt::Hyphenated::LENGTH + 4)
}
fn reason(value: &str) -> bool {
    text(value, capsem_proto::APPROVAL_CONTEXT_BYTES)
}
fn parent(value: &EnvironmentRevisionParent) -> bool {
    id(&value.environment_id) && id(&value.revision_id)
}
fn target(value: &EnvironmentRequestTarget) -> bool {
    id(&value.session_id) && id(&value.generation)
}

pub(super) fn revision(r: &EnvironmentRevisionRecord) -> bool {
    r.cursor > 0
        && id(&r.environment_id)
        && id(&r.revision_id)
        && r.previous_revision_id.as_deref().is_none_or(id)
        && r.fork_parent.as_ref().is_none_or(parent)
        && r.timestamp_unix_ms >= 0
        && actor(&r.actor)
        && reason(&r.reason)
        && r.request_id.as_deref().is_none_or(id)
        && valid_event_id(&r.payload_event_id)
}

pub(super) fn request(r: &EnvironmentRequestRecord) -> bool {
    if r.cursor <= 0
        || !id(&r.request_id)
        || !id(&r.environment_id)
        || !r.base_revision_id.as_deref().is_none_or(id)
        || !r.source.as_ref().is_none_or(parent)
        || !r.requester.as_ref().is_none_or(target)
        || r.timestamp_unix_ms < 0
        || r.expires_at_unix_ms <= r.timestamp_unix_ms
        || !actor(&r.actor)
        || !reason(&r.reason)
        || !valid_event_id(&r.payload_event_id)
        || !r.result.as_ref().is_none_or(parent)
        || r.application.as_ref().is_some_and(|a| {
            r.result.is_none()
                || r.requester.is_none()
                || match a.state {
                    EnvironmentApplicationState::Failed => !a.failure_reason.as_deref().is_some_and(reason),
                    _ => a.failure_reason.is_some(),
                }
        })
        || matches!(
            r.kind,
            EnvironmentRequestKind::Promotion | EnvironmentRequestKind::Apply
        ) != r.source.is_some()
    {
        return false;
    }
    if r.decision.as_ref().is_some_and(|d| {
        d.request_id != r.request_id
            || d.timestamp_unix_ms < r.timestamp_unix_ms
            || d.timestamp_unix_ms >= r.expires_at_unix_ms
            || !actor(&d.actor)
            || !d.decision.permits_actor(&d.actor, &r.actor)
            || !reason(&d.reason)
    }) {
        return false;
    }
    if r.invalidation.as_ref().is_some_and(|i| {
        i.request_id != r.request_id
            || i.timestamp_unix_ms < r.timestamp_unix_ms
            || !actor(&i.actor)
            || !reason(&i.reason)
            || r.decision
                .as_ref()
                .is_some_and(|d| d.timestamp_unix_ms > i.timestamp_unix_ms)
    }) {
        return false;
    }
    let decision = r.decision.as_ref().map(|d| d.decision);
    let invalidation = r.invalidation.as_ref().map(|i| i.kind);
    match r.state {
        EnvironmentRequestState::Pending => decision.is_none() && invalidation.is_none() && r.result.is_none(),
        EnvironmentRequestState::Approved => {
            decision == Some(EnvironmentRequestDecisionKind::Approve) && invalidation.is_none()
        }
        EnvironmentRequestState::Denied => {
            decision == Some(EnvironmentRequestDecisionKind::Deny) && invalidation.is_none() && r.result.is_none()
        }
        EnvironmentRequestState::Cancelled => {
            decision == Some(EnvironmentRequestDecisionKind::Cancel) && invalidation.is_none() && r.result.is_none()
        }
        EnvironmentRequestState::Expired => {
            decision.is_none()
                && r.result.is_none()
                && invalidation == Some(EnvironmentRequestInvalidationKind::Expired)
                && r.invalidation
                    .as_ref()
                    .is_some_and(|i| i.timestamp_unix_ms >= r.expires_at_unix_ms)
        }
        EnvironmentRequestState::Stale => {
            matches!(decision, None | Some(EnvironmentRequestDecisionKind::Approve))
                && r.result.is_none()
                && invalidation == Some(EnvironmentRequestInvalidationKind::Stale)
        }
    }
}

pub(super) fn request_payload(r: &EnvironmentRequestRecord, p: &EnvironmentRequestPayload) -> bool {
    if p.kind() != r.kind.as_str() || p.source_revision() != r.source.as_ref() {
        return false;
    }
    match p {
        EnvironmentRequestPayload::Change { specification }
        | EnvironmentRequestPayload::ImageUpdate { specification } => specification.validate().is_ok(),
        EnvironmentRequestPayload::Promotion { source } => parent(source),
        EnvironmentRequestPayload::Apply { revision, targets } => {
            let mut sessions = BTreeSet::new();
            parent(revision)
                && !targets.is_empty()
                && targets.len() <= capsem_proto::APPROVAL_PENDING_TOTAL
                && targets
                    .iter()
                    .all(|t| target(t) && sessions.insert(t.session_id.as_str()))
        }
    }
}
pub(super) fn request_key_absent(p: &EnvironmentRequestPayload, key: &str) -> bool {
    match p {
        EnvironmentRequestPayload::Change { specification }
        | EnvironmentRequestPayload::ImageUpdate { specification } => !specification.variables.contains_key(key),
        _ => false,
    }
}

pub(super) fn history<T>(
    replayable: bool,
    payload: &EnvironmentHistoryPayload<T>,
    valid: impl Fn(&T) -> bool,
    absent: impl Fn(&T, &str) -> bool,
) -> bool {
    match payload {
        EnvironmentHistoryPayload::Complete { value } => replayable && valid(value),
        EnvironmentHistoryPayload::Purged {
            value,
            purged_variables,
        } => {
            !replayable
                && !purged_variables.is_empty()
                && valid(value)
                && markers(purged_variables)
                && purged_variables.iter().all(|m| absent(value, &m.variable_name))
        }
        EnvironmentHistoryPayload::Unavailable { purged_variables } => !replayable && markers(purged_variables),
    }
}
fn markers(markers: &[crate::db::EnvironmentVariablePurgeMarker]) -> bool {
    let mut names = BTreeSet::new();
    markers.len() <= capsem_proto::ENVIRONMENT_MAX_VARIABLES
        && markers.iter().all(|m| {
            let key = m.variable_name.as_bytes();
            id(&m.purge_id)
                && m.timestamp_unix_ms >= 0
                && matches!(
                    m.state,
                    EnvironmentVariablePurgeState::Committed | EnvironmentVariablePurgeState::ReclaimPending
                )
                && key.len() <= capsem_proto::ENVIRONMENT_MAX_VARIABLE_NAME_BYTES
                && key.first().is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
                && key.iter().all(|b| b.is_ascii_alphanumeric() || *b == b'_')
                && capsem_proto::validate_env_key(&m.variable_name).is_ok()
                && names.insert(m.variable_name.as_str())
        })
}

pub(super) fn references(records: &[EnvironmentReferenceRecord]) -> bool {
    if records.len() > 3 {
        return false;
    }
    let mut dimensions = [false; 4];
    records.iter().all(|r| {
        let (index, revision) = match r {
            EnvironmentReferenceRecord::CurrentEnvironment { revision } => (0, revision),
            EnvironmentReferenceRecord::Sandbox { dimension, revision } => {
                let index = match dimension {
                    EnvironmentSandboxReferenceKind::Boot => 1,
                    EnvironmentSandboxReferenceKind::LivePolicy => 2,
                    EnvironmentSandboxReferenceKind::Desired => 3,
                };
                (index, revision)
            }
        };
        let unique = !dimensions[index];
        dimensions[index] = true;
        unique && parent(revision)
    }) && !(dimensions[0] && dimensions[1..].iter().any(|seen| *seen))
}

pub(super) fn application(r: &EnvironmentApplicationRecord) -> bool {
    let c = &r.composition;
    r.cursor > 0
        && r.version > 0
        && id(&r.request_id)
        && target(&r.target)
        && r.timestamp_unix_ms >= 0
        && actor(&r.actor)
        && parent(&c.boot)
        && parent(&c.live_policy)
        && parent(&c.desired)
        && reason(&c.corporate_policy_id)
        && !(c.pending_restart && c.boot == c.live_policy && c.boot == c.desired)
        && match r.state {
            EnvironmentApplicationState::Failed => r.failure_reason.as_deref().is_some_and(reason),
            _ => r.failure_reason.is_none(),
        }
}
