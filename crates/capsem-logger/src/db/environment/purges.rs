//! Durable purge intent/status; values stay exclusively in the existing archive.
use super::*;
use crate::events::environment::purge::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EnvironmentVariablePurgeQuery {
    Get {
        purge_id: String,
    },
    Page {
        environment_id: Option<String>,
        after_id: i64,
        limit: u16,
    },
}
impl EnvironmentVariablePurgeQuery {
    fn validate(&self) -> DbResult<()> {
        let valid = match self {
            Self::Get { purge_id } => valid_id(purge_id),
            Self::Page {
                environment_id,
                after_id,
                limit,
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
            Err("invalid environment purge query bounds or identity".into())
        }
    }
}
impl DbHandle {
    pub async fn environment_variable_purges(
        &self,
        query: EnvironmentVariablePurgeQuery,
    ) -> DbResult<Vec<EnvironmentVariablePurgeRecord>> {
        query.validate()?;
        let (reply, receiver) = tokio::sync::oneshot::channel();
        self.send_reader(ReadRequest::EnvironmentVariablePurges { query, reply })
            .map_err(|_| "environment purge reader worker closed")?;
        receiver
            .await
            .map_err(|_| "environment purge reader dropped reply")?
            .map(|observed| self.take_observed(observed))
    }
}
pub(in crate::db) fn execute(
    reader: &DbReader,
    query: &EnvironmentVariablePurgeQuery,
) -> DbResult<Vec<EnvironmentVariablePurgeRecord>> {
    let started = Instant::now();
    let _deadline = reader.sql_deadline(started + crate::db::bodies::INTERACTIVE_CAPTURE_DEADLINE);
    let result = query_rows(reader.connection(), query).map_err(|error| format!("environment purge query: {error}"));
    record_metadata_query("environment_variable_purge_execute", started, &result);
    result
}
pub(crate) const PURGE_COLUMNS:&str="p.id,p.purge_id,p.environment_id,p.source_revision_id,p.variable_name,p.timestamp_unix_ms,p.actor,p.state,p.old_generation_id,p.new_generation_id,p.failure_reason,(SELECT COUNT(*) FROM environment_revision_purges r WHERE r.purge_id=p.purge_id),(SELECT COUNT(*) FROM environment_request_purges q WHERE q.purge_id=p.purge_id)";
fn query_rows(
    conn: &rusqlite::Connection,
    query: &EnvironmentVariablePurgeQuery,
) -> rusqlite::Result<Vec<EnvironmentVariablePurgeRecord>> {
    match query {
        EnvironmentVariablePurgeQuery::Get { purge_id } => {
            let mut statement = conn.prepare_cached(&format!(
                "SELECT {PURGE_COLUMNS} FROM environment_variable_purges p WHERE p.purge_id=?1"
            ))?;
            let rows = statement.query_map([purge_id], decode_purge_row)?;
            rows.collect()
        }
        EnvironmentVariablePurgeQuery::Page {
            environment_id,
            after_id,
            limit,
        } => {
            let filter = if environment_id.is_some() {
                "p.environment_id=?1"
            } else {
                "?1 IS NULL"
            };
            let mut statement=conn.prepare_cached(&format!("SELECT {PURGE_COLUMNS} FROM environment_variable_purges p WHERE {filter} AND p.id>?2 ORDER BY p.id LIMIT ?3"))?;
            let rows = statement.query_map(rusqlite::params![environment_id, after_id, limit], decode_purge_row)?;
            rows.collect()
        }
    }
}
pub(crate) fn decode_purge_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<EnvironmentVariablePurgeRecord> {
    decode(row).map_err(rusqlite::Error::InvalidParameterName)
}
fn decode(row: &rusqlite::Row<'_>) -> DbResult<EnvironmentVariablePurgeRecord> {
    use crate::db::bodies::{bounded_optional_sql_text, bounded_sql_text};
    let text = |index, bound| bounded_sql_text(row, index, bound);
    let uuid = |index| {
        let value = text(index, uuid::fmt::Hyphenated::LENGTH)?;
        if !valid_id(&value) {
            return Err("invalid environment purge identity".to_owned());
        }
        Ok(value)
    };
    let integer = |index| {
        row.get::<_, i64>(index)
            .map_err(|error| format!("environment purge integer: {error}"))
    };
    let generation = |index| -> DbResult<Option<[u8; 16]>> {
        match row
            .get_ref(index)
            .map_err(|error| format!("environment purge generation: {error}"))?
        {
            rusqlite::types::ValueRef::Null => Ok(None),
            rusqlite::types::ValueRef::Blob(value) => {
                let bytes: [u8; 16] = value.try_into().map_err(|_| "invalid purge generation length")?;
                if bytes == [0; 16] {
                    return Err("invalid zero purge generation".into());
                }
                Ok(Some(bytes))
            }
            _ => Err("invalid environment purge generation type".into()),
        }
    };
    let record = EnvironmentVariablePurgeRecord {
        cursor: integer(0)?,
        intent: PreparedEnvironmentVariablePurge {
            purge_id: uuid(1)?,
            environment_id: uuid(2)?,
            source_revision_id: uuid(3)?,
            variable_name: text(4, capsem_proto::ENVIRONMENT_MAX_VARIABLE_NAME_BYTES)?,
            timestamp_unix_ms: integer(5)?,
            actor: text(6, "operator".len())?,
        },
        state: EnvironmentVariablePurgeState::parse_str(&text(7, 32)?).ok_or("unknown environment purge state")?,
        old_generation_id: generation(8)?,
        new_generation_id: generation(9)?,
        failure_reason: bounded_optional_sql_text(row, 10, capsem_proto::APPROVAL_CONTEXT_BYTES)?,
        revision_count: integer(11)?,
        request_count: integer(12)?,
    };
    if record.cursor <= 0
        || record.intent.validate().is_err()
        || record.revision_count <= 0
        || record.request_count < 0
        || record.old_generation_id.is_some() != record.new_generation_id.is_some()
        || record
            .old_generation_id
            .zip(record.new_generation_id)
            .is_some_and(|(old, new)| old == new)
        || record
            .failure_reason
            .as_ref()
            .is_some_and(|reason| reason.trim().is_empty() || reason.contains('\0'))
        || (record.state == EnvironmentVariablePurgeState::Prepared && record.old_generation_id.is_some())
        || (matches!(
            record.state,
            EnvironmentVariablePurgeState::Rewriting
                | EnvironmentVariablePurgeState::ReclaimPending
                | EnvironmentVariablePurgeState::Committed
        ) && record.old_generation_id.is_none())
    {
        return Err("invalid environment purge metadata".into());
    }
    Ok(record)
}
