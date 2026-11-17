//! Typed environment history through the existing DB-owned reader thread.
use super::*;
use crate::events::{EnvironmentRevisionParent, EnvironmentRevisionState};
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EnvironmentRevisionQuery {
    Get {
        environment_id: String,
        revision_id: String,
    },
    Page {
        environment_id: Option<String>,
        state: Option<EnvironmentRevisionState>,
        after_id: i64,
        limit: u16,
    },
}

fn valid_id(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| id.to_string() == value && !id.is_nil())
}

impl EnvironmentRevisionQuery {
    fn validate(&self) -> DbResult<()> {
        let valid = match self {
            Self::Get {
                environment_id,
                revision_id,
            } => valid_id(environment_id) && valid_id(revision_id),
            Self::Page {
                environment_id,
                after_id,
                limit,
                ..
            } => {
                environment_id.as_deref().is_none_or(valid_id)
                    && *after_id >= 0
                    && *limit > 0
                    && usize::from(*limit) <= crate::ledger_protocol::MAX_LEDGER_QUERY_ROWS
            }
        };
        if valid {
            Ok(())
        } else {
            Err("invalid environment revision query bounds or identity".into())
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentRevisionRecord {
    pub cursor: i64,
    pub environment_id: String,
    pub revision_id: String,
    pub previous_revision_id: Option<String>,
    pub fork_parent: Option<EnvironmentRevisionParent>,
    pub timestamp_unix_ms: i64,
    pub actor: String,
    pub reason: String,
    pub request_id: Option<String>,
    pub state: EnvironmentRevisionState,
    pub payload_event_id: String,
    pub replayable: bool,
}

impl std::fmt::Debug for EnvironmentRevisionRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvironmentRevisionRecord")
            .field("cursor", &self.cursor)
            .field("environment_id", &self.environment_id)
            .field("revision_id", &self.revision_id)
            .field("state", &self.state)
            .field("replayable", &self.replayable)
            .finish_non_exhaustive()
    }
}

impl DbHandle {
    /// Durable metadata in exclusive cursor order; Get returns zero or one row.
    /// Snapshot bytes remain in the archive, never in this metadata projection.
    pub async fn environment_revisions(
        &self,
        query: EnvironmentRevisionQuery,
    ) -> DbResult<Vec<EnvironmentRevisionRecord>> {
        query.validate()?;
        let (reply, receiver) = tokio::sync::oneshot::channel();
        self.send_reader(ReadRequest::EnvironmentRevisions { query, reply })
            .map_err(|_| "environment revision reader worker closed")?;
        receiver
            .await
            .map_err(|_| "environment revision reader dropped reply")?
            .map(|observed| self.take_observed(observed))
    }
}

pub(super) fn execute(reader: &DbReader, query: &EnvironmentRevisionQuery) -> DbResult<Vec<EnvironmentRevisionRecord>> {
    let started = Instant::now();
    let result = query_rows(reader, query).map_err(|error| format!("environment revision query: {error}"));
    let status = if result.is_ok() { "ok" } else { "error" };
    ::metrics::counter!(DB_QUERY_TOTAL, "phase" => "environment_revision_execute", "status" => status).increment(1);
    ::metrics::histogram!(DB_QUERY_DURATION_MS, "phase" => "environment_revision_execute", "status" => status)
        .record(elapsed_ms_f64(started));
    match &result {
        Ok(rows) => {
            ::metrics::histogram!(DB_QUERY_RESULT_ROWS, "phase" => "environment_revision_execute")
                .record(rows.len() as f64);
            tracing::debug!(
                operation = "environment_revision_query",
                rows = rows.len(),
                duration_ms = elapsed_ms(started),
                "environment revision metadata read"
            );
        }
        Err(error) => {
            tracing::error!(operation = "environment_revision_query", error = %error, duration_ms = elapsed_ms(started),
            "environment revision metadata read failed")
        }
    }
    result
}

const COLUMNS: &str = "id,environment_id,revision_id,previous_revision_id,fork_parent_environment_id,fork_parent_revision_id,timestamp_unix_ms,actor,reason,request_id,state,payload_event_id,replayable";

fn query_rows(reader: &DbReader, query: &EnvironmentRevisionQuery) -> rusqlite::Result<Vec<EnvironmentRevisionRecord>> {
    match query {
        EnvironmentRevisionQuery::Get {
            environment_id,
            revision_id,
        } => {
            let mut statement = reader.connection().prepare_cached(&format!(
                "SELECT {COLUMNS} FROM environment_revisions WHERE environment_id=?1 AND revision_id=?2"
            ))?;
            let rows = statement.query_map(rusqlite::params![environment_id, revision_id], from_row)?;
            rows.collect()
        }
        EnvironmentRevisionQuery::Page {
            environment_id,
            state,
            after_id,
            limit,
        } => {
            let environment = if environment_id.is_some() {
                "environment_id=?1"
            } else {
                "?1 IS NULL"
            };
            // Literal enum clauses let SQLite use the existing partial inflight
            // index for startup recovery; no caller-supplied SQL is interpolated.
            let state = match state {
                None => "",
                Some(EnvironmentRevisionState::Committing) => " AND state='committing'",
                Some(EnvironmentRevisionState::Committed) => " AND state='committed'",
                Some(EnvironmentRevisionState::RecoveredUnapplied) => " AND state='recovered_unapplied'",
            };
            let mut statement = reader.connection().prepare_cached(&format!(
                "SELECT {COLUMNS} FROM environment_revisions WHERE {environment}{state} AND id>?2 ORDER BY id LIMIT ?3"
            ))?;
            let rows = statement.query_map(rusqlite::params![environment_id, after_id, limit], from_row)?;
            rows.collect()
        }
    }
}

fn invalid(message: &'static str) -> rusqlite::Error {
    rusqlite::Error::InvalidParameterName(message.into())
}

fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<EnvironmentRevisionRecord> {
    let parent_environment: Option<String> = row.get(4)?;
    let parent_revision: Option<String> = row.get(5)?;
    let fork_parent = match (parent_environment, parent_revision) {
        (Some(environment_id), Some(revision_id)) => Some(EnvironmentRevisionParent {
            environment_id,
            revision_id,
        }),
        (None, None) => None,
        _ => return Err(invalid("incomplete environment revision fork lineage")),
    };
    let state: String = row.get(10)?;
    let state =
        EnvironmentRevisionState::parse_str(&state).ok_or_else(|| invalid("unknown environment revision state"))?;
    let replayable = match row.get::<_, i64>(12)? {
        0 => false,
        1 => true,
        _ => return Err(invalid("invalid environment revision replayable marker")),
    };
    Ok(EnvironmentRevisionRecord {
        cursor: row.get(0)?,
        environment_id: row.get(1)?,
        revision_id: row.get(2)?,
        previous_revision_id: row.get(3)?,
        fork_parent,
        timestamp_unix_ms: row.get(6)?,
        actor: row.get(7)?,
        reason: row.get(8)?,
        request_id: row.get(9)?,
        state,
        payload_event_id: row.get(11)?,
        replayable,
    })
}
