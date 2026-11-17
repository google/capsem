//! Restart-safe request views through the DB-owned reader and archive lease.
use super::history::{self, EnvironmentHistoryPayload, EnvironmentVariablePurgeMarker, SnapshotMode};
use super::*;
use crate::events::environment::request::{
    EnvironmentRequestDecision, EnvironmentRequestDecisionKind, EnvironmentRequestInvalidation,
    EnvironmentRequestInvalidationKind, EnvironmentRequestKind, EnvironmentRequestPayload, EnvironmentRequestState,
    EnvironmentRequestTarget,
};

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EnvironmentRequestQuery {
    Get {
        request_id: String,
    },
    Page {
        environment_id: Option<String>,
        requester: Option<EnvironmentRequestTarget>,
        pending_only: bool,
        after_id: i64,
        limit: u16,
    },
}

impl EnvironmentRequestQuery {
    pub(crate) fn validate(&self) -> DbResult<()> {
        let valid = match self {
            Self::Get { request_id } => valid_id(request_id),
            Self::Page {
                environment_id,
                requester,
                after_id,
                limit,
                ..
            } => {
                environment_id.as_deref().is_none_or(valid_id)
                    && requester
                        .as_ref()
                        .is_none_or(|value| valid_id(&value.session_id) && valid_id(&value.generation))
                    && *after_id >= 0
                    && *limit > 0
                    && usize::from(*limit) <= crate::ledger_protocol::MAX_LEDGER_QUERY_ROWS
            }
        };
        if valid {
            Ok(())
        } else {
            Err("invalid environment request query bounds or identity".into())
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentRequestRecord {
    pub cursor: i64,
    pub request_id: String,
    pub kind: EnvironmentRequestKind,
    pub environment_id: String,
    pub base_revision_id: Option<String>,
    pub source: Option<crate::events::EnvironmentRevisionParent>,
    pub requester: Option<EnvironmentRequestTarget>,
    pub timestamp_unix_ms: i64,
    pub expires_at_unix_ms: i64,
    pub actor: String,
    pub reason: String,
    pub state: EnvironmentRequestState,
    pub payload_event_id: String,
    pub replayable: bool,
    pub decision: Option<EnvironmentRequestDecision>,
    pub invalidation: Option<EnvironmentRequestInvalidation>,
    pub result: Option<crate::events::EnvironmentRevisionParent>,
}
impl std::fmt::Debug for EnvironmentRequestRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvironmentRequestRecord")
            .field("cursor", &self.cursor)
            .field("request_id", &self.request_id)
            .field("kind", &self.kind)
            .field("state", &self.state)
            .field("replayable", &self.replayable)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentRequestSnapshot {
    pub record: EnvironmentRequestRecord,
    pub payload: Box<EnvironmentRequestPayload>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentRequestHistory {
    pub record: EnvironmentRequestRecord,
    pub payload: EnvironmentHistoryPayload<EnvironmentRequestPayload>,
}

impl DbHandle {
    pub async fn environment_requests(
        &self,
        query: EnvironmentRequestQuery,
    ) -> DbResult<Vec<EnvironmentRequestRecord>> {
        query.validate()?;
        let (reply, receiver) = tokio::sync::oneshot::channel();
        self.send_reader(ReadRequest::EnvironmentRequests { query, reply })
            .map_err(|_| "environment request reader worker closed")?;
        receiver
            .await
            .map_err(|_| "environment request reader dropped reply")?
            .map(|observed| self.take_observed(observed))
    }
    pub async fn environment_request_snapshot(&self, request_id: &str) -> DbResult<Option<EnvironmentRequestSnapshot>> {
        let Some(history) = self.request_history(request_id, SnapshotMode::Replay).await? else {
            return Ok(None);
        };
        let EnvironmentHistoryPayload::Complete { value } = history.payload else {
            return Err("environment request is non-replayable".into());
        };
        Ok(Some(EnvironmentRequestSnapshot {
            record: history.record,
            payload: value,
        }))
    }

    pub async fn environment_request_history(&self, request_id: &str) -> DbResult<Option<EnvironmentRequestHistory>> {
        self.request_history(request_id, SnapshotMode::History).await
    }

    async fn request_history(
        &self,
        request_id: &str,
        mode: SnapshotMode,
    ) -> DbResult<Option<EnvironmentRequestHistory>> {
        let query = EnvironmentRequestQuery::Get {
            request_id: request_id.into(),
        };
        query.validate()?;
        let (reply, receiver) = tokio::sync::oneshot::channel();
        self.send_reader(ReadRequest::EnvironmentRequestSnapshot { query, mode, reply })
            .map_err(|_| "environment request snapshot reader closed")?;
        let Some(captured) = self.take_observed(
            receiver
                .await
                .map_err(|_| "environment request snapshot reader dropped reply")??,
        ) else {
            return Ok(None);
        };
        let handle = self.clone();
        tokio::task::spawn_blocking(move || {
            let CapturedRequest {
                reader,
                record,
                index,
                purges,
            } = captured;
            let value = if let Some(index) = index {
                let body = handle
                    .read_archived_blocking(crate::db::bodies::CapturedBodies {
                        reader,
                        rows: vec![index],
                    })?
                    .pop()
                    .ok_or("required environment request payload is missing")?;
                let payload: EnvironmentRequestPayload =
                    rmp_serde::from_slice(&body.bytes).map_err(|_| "invalid archived environment request encoding")?;
                if payload.source_revision() != record.source.as_ref() {
                    return Err("environment request source metadata does not match its archived intent".into());
                }
                if payload.kind() != record.kind.as_str() {
                    return Err("environment request kind does not match its archived intent".into());
                }
                match &payload {
                    EnvironmentRequestPayload::Change { specification }
                    | EnvironmentRequestPayload::ImageUpdate { specification } => {
                        specification
                            .validate()
                            .map_err(|_| "invalid archived environment request specification")?;
                        if purges
                            .iter()
                            .any(|marker| specification.variables.contains_key(&marker.variable_name))
                        {
                            return Err("archived proposal values contradict published purge markers".into());
                        }
                    }
                    _ if !purges.is_empty() => return Err("reference-only proposal has variable purge markers".into()),
                    _ => (),
                }
                Some(Box::new(payload))
            } else {
                None
            };
            let payload = history::payload(record.replayable, value, purges)?;
            Ok(Some(EnvironmentRequestHistory { record, payload }))
        })
        .await
        .map_err(|_| "environment request reconstruction worker failed")?
    }
}

const COLUMNS: &str = "r.id,r.request_id,r.kind,r.environment_id,r.base_revision_id,r.requester_session_id,r.requester_generation,r.timestamp_unix_ms,r.actor,r.reason,r.state,r.payload_event_id,r.replayable,r.result_environment_id,r.result_revision_id,d.timestamp_unix_ms,d.actor,d.decision,d.reason,r.source_environment_id,r.source_revision_id,r.expires_at_unix_ms,i.timestamp_unix_ms,i.actor,i.kind,i.reason";
const TABLES: &str = "environment_requests r LEFT JOIN environment_request_decisions d ON d.request_id=r.request_id LEFT JOIN environment_request_invalidations i ON i.request_id=r.request_id";
const UUID_BYTES: usize = uuid::fmt::Hyphenated::LENGTH;
// Longest canonical actor is vm:<session UUID>/<generation UUID>.
const ACTOR_BYTES: usize = 2 * UUID_BYTES + 4;

pub(in crate::db) fn execute(
    reader: &DbReader,
    query: &EnvironmentRequestQuery,
) -> DbResult<Vec<EnvironmentRequestRecord>> {
    let started = Instant::now();
    let _deadline = reader.sql_deadline(started + crate::db::bodies::INTERACTIVE_CAPTURE_DEADLINE);
    let result = query_rows(reader.connection(), query);
    record_metadata_query("environment_request_execute", started, &result);
    result
}

fn query_rows(
    connection: &rusqlite::Connection,
    query: &EnvironmentRequestQuery,
) -> DbResult<Vec<EnvironmentRequestRecord>> {
    match query {
        EnvironmentRequestQuery::Get { request_id } => {
            let mut statement = connection
                .prepare_cached(&format!("SELECT {COLUMNS} FROM {TABLES} WHERE r.request_id=?1"))
                .map_err(|error| format!("prepare environment request query: {error}"))?;
            let rows = statement
                .query([request_id])
                .map_err(|error| format!("execute environment request query: {error}"))?;
            collect_rows(rows)
        }
        EnvironmentRequestQuery::Page {
            environment_id,
            requester,
            pending_only,
            after_id,
            limit,
        } => {
            let environment = if environment_id.is_some() {
                "r.environment_id=?1"
            } else {
                "?1 IS NULL"
            };
            let requester_filter = if requester.is_some() {
                "r.requester_session_id=?2 AND r.requester_generation=?3"
            } else {
                "?2 IS NULL AND ?3 IS NULL"
            };
            let pending = if *pending_only { "AND r.state='pending'" } else { "" };
            let mut statement = connection.prepare_cached(&format!("SELECT {COLUMNS} FROM {TABLES} WHERE {environment} AND {requester_filter} {pending} AND r.id>?4 ORDER BY r.id LIMIT ?5"))
                .map_err(|error| format!("prepare environment request page: {error}"))?;
            let rows = statement
                .query(rusqlite::params![
                    environment_id,
                    requester.as_ref().map(|value| &value.session_id),
                    requester.as_ref().map(|value| &value.generation),
                    after_id,
                    limit
                ])
                .map_err(|error| format!("execute environment request page: {error}"))?;
            collect_rows(rows)
        }
    }
}

fn collect_rows(mut rows: rusqlite::Rows<'_>) -> DbResult<Vec<EnvironmentRequestRecord>> {
    let mut records = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|error| format!("read environment request row: {error}"))?
    {
        records.push(from_row(row)?);
    }
    Ok(records)
}

fn from_row(row: &rusqlite::Row<'_>) -> DbResult<EnvironmentRequestRecord> {
    use crate::db::bodies::{bounded_optional_sql_text, bounded_sql_text};
    let text = |index, bound| bounded_sql_text(row, index, bound);
    let optional = |index, bound| bounded_optional_sql_text(row, index, bound);
    let request_id = text(1, UUID_BYTES)?;
    let kind = EnvironmentRequestKind::parse_str(&text(2, capsem_proto::APPROVAL_CONTEXT_BYTES)?)
        .ok_or("unknown environment request kind")?;
    let state = EnvironmentRequestState::parse_str(&text(10, capsem_proto::APPROVAL_CONTEXT_BYTES)?)
        .ok_or("unknown environment request state")?;
    let requester = match (optional(5, UUID_BYTES)?, optional(6, UUID_BYTES)?) {
        (Some(session_id), Some(generation)) => Some(EnvironmentRequestTarget { session_id, generation }),
        (None, None) => None,
        _ => return Err("incomplete environment requester identity".into()),
    };
    let result = match (optional(13, UUID_BYTES)?, optional(14, UUID_BYTES)?) {
        (Some(environment_id), Some(revision_id)) => Some(crate::events::EnvironmentRevisionParent {
            environment_id,
            revision_id,
        }),
        (None, None) => None,
        _ => return Err("incomplete environment request result identity".into()),
    };
    let source = match (optional(19, UUID_BYTES)?, optional(20, UUID_BYTES)?) {
        (Some(environment_id), Some(revision_id)) => Some(crate::events::EnvironmentRevisionParent {
            environment_id,
            revision_id,
        }),
        (None, None) => None,
        _ => return Err("incomplete environment request source identity".into()),
    };
    if matches!(kind, EnvironmentRequestKind::Promotion | EnvironmentRequestKind::Apply) != source.is_some() {
        return Err("environment request kind does not match its source metadata".into());
    }
    let at: Option<i64> = row
        .get(15)
        .map_err(|error| format!("environment decision timestamp: {error}"))?;
    let decision = match (
        at,
        optional(16, ACTOR_BYTES)?,
        optional(17, capsem_proto::APPROVAL_CONTEXT_BYTES)?,
        optional(18, capsem_proto::APPROVAL_CONTEXT_BYTES)?,
    ) {
        (None, None, None, None) => None,
        (Some(timestamp_unix_ms), Some(actor), Some(kind), Some(reason)) => {
            let decision = [
                EnvironmentRequestDecisionKind::Approve,
                EnvironmentRequestDecisionKind::Deny,
                EnvironmentRequestDecisionKind::Cancel,
            ]
            .into_iter()
            .find(|value| value.as_str() == kind)
            .ok_or("unknown environment request decision")?;
            Some(EnvironmentRequestDecision {
                request_id: request_id.clone(),
                timestamp_unix_ms,
                actor,
                decision,
                reason,
            })
        }
        _ => return Err("incomplete environment request decision evidence".into()),
    };
    let created_at: i64 = row
        .get(7)
        .map_err(|error| format!("environment request timestamp: {error}"))?;
    let expires_at_unix_ms: i64 = row
        .get(21)
        .map_err(|error| format!("environment request expiry: {error}"))?;
    let invalidated_at: Option<i64> = row
        .get(22)
        .map_err(|error| format!("environment invalidation timestamp: {error}"))?;
    let invalidation = match (
        invalidated_at,
        optional(23, ACTOR_BYTES)?,
        optional(24, 32)?,
        optional(25, capsem_proto::APPROVAL_CONTEXT_BYTES)?,
    ) {
        (None, None, None, None) => None,
        (Some(timestamp_unix_ms), Some(actor), Some(kind), Some(reason)) => Some(EnvironmentRequestInvalidation {
            request_id: request_id.clone(),
            timestamp_unix_ms,
            actor,
            kind: EnvironmentRequestInvalidationKind::parse_str(&kind)
                .ok_or("unknown environment request invalidation")?,
            reason,
        }),
        _ => return Err("incomplete environment request invalidation evidence".into()),
    };
    let decided = decision.as_ref().map(|value| value.decision);
    let invalidated = invalidation.as_ref().map(|value| value.kind);
    let coherent = match state {
        EnvironmentRequestState::Pending => decided.is_none() && invalidated.is_none() && result.is_none(),
        EnvironmentRequestState::Approved => {
            decided == Some(EnvironmentRequestDecisionKind::Approve) && invalidated.is_none()
        }
        EnvironmentRequestState::Denied => {
            decided == Some(EnvironmentRequestDecisionKind::Deny) && invalidated.is_none() && result.is_none()
        }
        EnvironmentRequestState::Cancelled => {
            decided == Some(EnvironmentRequestDecisionKind::Cancel) && invalidated.is_none() && result.is_none()
        }
        EnvironmentRequestState::Expired => {
            decided.is_none()
                && invalidated == Some(EnvironmentRequestInvalidationKind::Expired)
                && result.is_none()
                && invalidation
                    .as_ref()
                    .is_some_and(|value| value.timestamp_unix_ms >= expires_at_unix_ms)
        }
        EnvironmentRequestState::Stale => {
            (decided.is_none() || decided == Some(EnvironmentRequestDecisionKind::Approve))
                && invalidated == Some(EnvironmentRequestInvalidationKind::Stale)
                && result.is_none()
        }
    };
    if !coherent
        || created_at < 0
        || expires_at_unix_ms <= created_at
        || decision
            .as_ref()
            .is_some_and(|value| value.timestamp_unix_ms < created_at || value.timestamp_unix_ms >= expires_at_unix_ms)
        || invalidation.as_ref().is_some_and(|value| {
            value.timestamp_unix_ms < created_at
                || decision
                    .as_ref()
                    .is_some_and(|decision| value.timestamp_unix_ms < decision.timestamp_unix_ms)
        })
    {
        return Err("environment request state does not match its lifecycle evidence".into());
    }
    let replayable = match row
        .get::<_, i64>(12)
        .map_err(|error| format!("environment request replayability: {error}"))?
    {
        0 => false,
        1 => true,
        _ => return Err("invalid environment request replayability marker".into()),
    };
    Ok(EnvironmentRequestRecord {
        cursor: row
            .get(0)
            .map_err(|error| format!("environment request cursor: {error}"))?,
        request_id,
        kind,
        environment_id: text(3, UUID_BYTES)?,
        base_revision_id: optional(4, UUID_BYTES)?,
        source,
        requester,
        timestamp_unix_ms: row
            .get(7)
            .map_err(|error| format!("environment request timestamp: {error}"))?,
        actor: text(8, ACTOR_BYTES)?,
        reason: text(9, capsem_proto::APPROVAL_CONTEXT_BYTES)?,
        state,
        payload_event_id: text(11, 12)?,
        replayable,
        decision,
        invalidation,
        expires_at_unix_ms,
        result,
    })
}

pub(in crate::db) struct CapturedRequest {
    reader: capsem_archive::BodyLogReader,
    record: EnvironmentRequestRecord,
    index: Option<crate::db::bodies::IndexRow>,
    purges: Vec<EnvironmentVariablePurgeMarker>,
}

pub(in crate::db) fn capture(
    reader: &DbReader,
    path: &std::path::Path,
    query: &EnvironmentRequestQuery,
    mode: SnapshotMode,
    codecs: &capsem_archive::ArchiveCodecs,
) -> DbResult<Option<CapturedRequest>> {
    if !matches!(query, EnvironmentRequestQuery::Get { .. }) {
        return Err("environment request snapshot requires exact identity".into());
    }
    let (reader, selection) =
        crate::db::bodies::with_archive_snapshot(reader, path, codecs, |connection, deadline| {
            deadline.check("environment request capture")?;
            let Some(record) = query_rows(connection, query)?.pop() else {
                return Ok(None);
            };
            let purges = history::request_purges(connection, &record.request_id)?;
            let index = history::index(
                connection,
                &record.payload_event_id,
                "environment_requests",
                record.replayable,
                mode,
                &purges,
            )?;
            Ok(Some((record, index, purges)))
        })?;
    Ok(selection.map(|(record, index, purges)| CapturedRequest {
        reader,
        record,
        index,
        purges,
    }))
}
