//! Read-only historical views never become replay authority.
use super::*;
use crate::db::bodies::{bounded_sql_text, IndexRow};
use crate::events::environment::purge::{EnvironmentVariablePurgeState, PreparedEnvironmentVariablePurge};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EnvironmentHistoryPayload<T> {
    Complete {
        value: Box<T>,
    },
    Purged {
        value: Box<T>,
        purged_variables: Vec<EnvironmentVariablePurgeMarker>,
    },
    /// The retained metadata survives even when its payload has expired.
    Unavailable {
        purged_variables: Vec<EnvironmentVariablePurgeMarker>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentVariablePurgeMarker {
    pub purge_id: String,
    pub variable_name: String,
    /// Time of the operator intent, rather than an invented completion time.
    pub timestamp_unix_ms: i64,
    pub state: EnvironmentVariablePurgeState,
}

#[derive(Clone, Copy)]
pub(in crate::db) enum SnapshotMode {
    Replay,
    History,
}

pub(super) fn revision_purges(
    conn: &rusqlite::Connection,
    revision_id: &str,
) -> DbResult<Vec<EnvironmentVariablePurgeMarker>> {
    purges(conn, revision_id, MarkerSubject::Revision)
}

pub(super) fn request_purges(
    conn: &rusqlite::Connection,
    request_id: &str,
) -> DbResult<Vec<EnvironmentVariablePurgeMarker>> {
    purges(conn, request_id, MarkerSubject::Request)
}

enum MarkerSubject {
    Revision,
    Request,
}

fn purges(
    conn: &rusqlite::Connection,
    owner_id: &str,
    subject: MarkerSubject,
) -> DbResult<Vec<EnvironmentVariablePurgeMarker>> {
    let (link, column) = match subject {
        MarkerSubject::Revision => ("environment_revision_purges", "revision_id"),
        MarkerSubject::Request => ("environment_request_purges", "request_id"),
    };
    let mut statement = conn.prepare_cached(&format!("SELECT p.purge_id,p.environment_id,p.source_revision_id,p.variable_name,p.timestamp_unix_ms,p.actor,p.state
        FROM {link} m JOIN environment_variable_purges p ON p.purge_id=m.purge_id
        WHERE m.{column}=?1 AND p.state IN ('reclaim_pending','committed') ORDER BY p.id LIMIT ?2"))
        .map_err(|error| format!("environment history purge markers: {error}"))?;
    let mut rows = statement
        .query(rusqlite::params![
            owner_id,
            (capsem_proto::ENVIRONMENT_MAX_VARIABLES + 1) as i64
        ])
        .map_err(|error| format!("environment history purge markers: {error}"))?;
    let mut markers = Vec::new();
    let mut names = BTreeSet::new();
    while let Some(row) = rows
        .next()
        .map_err(|_| "cannot read environment history purge markers")?
    {
        if markers.len() == capsem_proto::ENVIRONMENT_MAX_VARIABLES {
            return Err("environment history exceeds its variable marker budget".into());
        }
        let intent = PreparedEnvironmentVariablePurge {
            purge_id: bounded_sql_text(row, 0, 36)?,
            environment_id: bounded_sql_text(row, 1, 36)?,
            source_revision_id: bounded_sql_text(row, 2, 36)?,
            variable_name: bounded_sql_text(row, 3, capsem_proto::ENVIRONMENT_MAX_VARIABLE_NAME_BYTES)?,
            timestamp_unix_ms: row.get(4).map_err(|_| "invalid environment history purge time")?,
            actor: bounded_sql_text(row, 5, 64)?,
        };
        intent.validate().map_err(str::to_owned)?;
        let state = EnvironmentVariablePurgeState::parse_str(&bounded_sql_text(row, 6, 32)?)
            .ok_or("invalid environment history purge phase")?;
        if !matches!(
            state,
            EnvironmentVariablePurgeState::Committed | EnvironmentVariablePurgeState::ReclaimPending
        ) || !names.insert(intent.variable_name.clone())
        {
            return Err("invalid or repeated published variable purge marker".into());
        }
        markers.push(EnvironmentVariablePurgeMarker {
            purge_id: intent.purge_id,
            variable_name: intent.variable_name,
            timestamp_unix_ms: intent.timestamp_unix_ms,
            state,
        });
    }
    Ok(markers)
}

pub(super) fn index(
    conn: &rusqlite::Connection,
    event_id: &str,
    source: &'static str,
    replayable: bool,
    mode: SnapshotMode,
    markers: &[EnvironmentVariablePurgeMarker],
) -> DbResult<Option<IndexRow>> {
    if replayable && !markers.is_empty() {
        return Err("replayable environment history contradicts its published purge markers".into());
    }
    if !replayable && matches!(mode, SnapshotMode::Replay) {
        return Err("environment history is non-replayable because required historical values are unavailable".into());
    }
    if !replayable {
        let present:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM event_body_blobs WHERE event_id=?1 AND source_table=?2 AND direction='payload')",[event_id,source],|row|row.get(0))
            .map_err(|error|format!("environment history payload lookup: {error}"))?;
        if !present {
            return Ok(None);
        }
        if markers.is_empty() {
            return Err("non-replayable environment payload lacks published purge markers".into());
        }
    }
    crate::db::bodies::required_configuration_index(conn, event_id, source).map(Some)
}

pub(super) fn payload<T>(
    replayable: bool,
    value: Option<Box<T>>,
    markers: Vec<EnvironmentVariablePurgeMarker>,
) -> DbResult<EnvironmentHistoryPayload<T>> {
    match (replayable, value) {
        (true, Some(value)) => Ok(EnvironmentHistoryPayload::Complete { value }),
        (false, Some(value)) => Ok(EnvironmentHistoryPayload::Purged {
            value,
            purged_variables: markers,
        }),
        (false, None) => Ok(EnvironmentHistoryPayload::Unavailable {
            purged_variables: markers,
        }),
        (true, None) => Err("replayable environment history payload is missing".into()),
    }
}
