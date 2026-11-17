//! Indexed application results through the existing reader worker.
use super::*;
use crate::events::environment::application::*;
use crate::events::environment::request::EnvironmentRequestTarget;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EnvironmentApplicationQuery {
    Get {
        request_id: String,
        target: EnvironmentRequestTarget,
    },
    Page {
        request_id: Option<String>,
        target: Option<EnvironmentRequestTarget>,
        after_id: i64,
        limit: u16,
    },
}
impl EnvironmentApplicationQuery {
    fn validate(&self) -> DbResult<()> {
        let target_valid =
            |target: &EnvironmentRequestTarget| valid_id(&target.session_id) && valid_id(&target.generation);
        let valid = match self {
            Self::Get { request_id, target } => valid_id(request_id) && target_valid(target),
            Self::Page {
                request_id,
                target,
                after_id,
                limit,
            } => {
                request_id.as_deref().is_none_or(valid_id)
                    && target.as_ref().is_none_or(target_valid)
                    && *after_id >= 0
                    && *limit > 0
                    && usize::from(*limit) <= crate::ledger_protocol::MAX_LEDGER_QUERY_ROWS
            }
        };
        if valid {
            Ok(())
        } else {
            Err("invalid environment application query bounds or identity".into())
        }
    }
}
impl DbHandle {
    pub async fn environment_applications(
        &self,
        query: EnvironmentApplicationQuery,
    ) -> DbResult<Vec<EnvironmentApplicationRecord>> {
        query.validate()?;
        let (reply, receiver) = tokio::sync::oneshot::channel();
        self.send_reader(ReadRequest::EnvironmentApplications { query, reply })
            .map_err(|_| "environment application reader worker closed")?;
        receiver
            .await
            .map_err(|_| "environment application reader dropped reply")?
            .map(|observed| self.take_observed(observed))
    }
}

pub(in crate::db) fn execute(
    reader: &DbReader,
    query: &EnvironmentApplicationQuery,
) -> DbResult<Vec<EnvironmentApplicationRecord>> {
    let started = Instant::now();
    let _deadline = reader.sql_deadline(started + crate::db::bodies::INTERACTIVE_CAPTURE_DEADLINE);
    let result =
        query_rows(reader.connection(), query).map_err(|error| format!("environment application query: {error}"));
    record_metadata_query("environment_application_execute", started, &result);
    result
}

pub(crate) const APPLICATION_COLUMNS:&str="id,version,request_id,session_id,generation,timestamp_unix_ms,actor,state,boot_environment_id,boot_revision_id,live_policy_environment_id,live_policy_revision_id,desired_environment_id,desired_revision_id,corporate_policy_id,pending_restart,failure_reason";

fn query_rows(
    conn: &rusqlite::Connection,
    query: &EnvironmentApplicationQuery,
) -> rusqlite::Result<Vec<EnvironmentApplicationRecord>> {
    match query {
        EnvironmentApplicationQuery::Get { request_id, target } => {
            let mut statement=conn.prepare_cached(&format!("SELECT {APPLICATION_COLUMNS} FROM environment_applications WHERE request_id=?1 AND session_id=?2 AND generation=?3"))?;
            let rows = statement.query_map(
                rusqlite::params![request_id, target.session_id, target.generation],
                decode_application_row,
            )?;
            rows.collect()
        }
        EnvironmentApplicationQuery::Page {
            request_id,
            target,
            after_id,
            limit,
        } => {
            let request_filter = if request_id.is_some() {
                "request_id=?1"
            } else {
                "?1 IS NULL"
            };
            let target_filter = if target.is_some() {
                "session_id=?2 AND generation=?3"
            } else {
                "?2 IS NULL AND ?3 IS NULL"
            };
            let mut statement=conn.prepare_cached(&format!("SELECT {APPLICATION_COLUMNS} FROM environment_applications WHERE {request_filter} AND {target_filter} AND id>?4 ORDER BY id LIMIT ?5"))?;
            let rows = statement.query_map(
                rusqlite::params![
                    request_id,
                    target.as_ref().map(|value| &value.session_id),
                    target.as_ref().map(|value| &value.generation),
                    after_id,
                    limit
                ],
                decode_application_row,
            )?;
            rows.collect()
        }
    }
}

/// Shared typed row decoding for reader execution and writer CAS checks.
pub(crate) fn decode_application_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<EnvironmentApplicationRecord> {
    decode(row).map_err(rusqlite::Error::InvalidParameterName)
}
fn decode(row: &rusqlite::Row<'_>) -> DbResult<EnvironmentApplicationRecord> {
    use crate::db::bodies::{bounded_optional_sql_text, bounded_sql_text};
    let integer = |index| {
        row.get::<_, i64>(index)
            .map_err(|error| format!("environment application integer: {error}"))
    };
    let uuid = |index| {
        let value = bounded_sql_text(row, index, uuid::fmt::Hyphenated::LENGTH)?;
        if !valid_id(&value) {
            return Err("invalid environment application identity".to_owned());
        }
        Ok(value)
    };
    let reference = |index| {
        Ok::<_, String>(EnvironmentRevisionParent {
            environment_id: uuid(index)?,
            revision_id: uuid(index + 1)?,
        })
    };
    let state = EnvironmentApplicationState::parse_str(&bounded_sql_text(row, 7, 32)?)
        .ok_or("unknown environment application state")?;
    let pending_restart = match integer(15)? {
        0 => false,
        1 => true,
        _ => return Err("invalid application pending-restart marker".into()),
    };
    let record = EnvironmentApplicationRecord {
        cursor: integer(0)?,
        version: integer(1)?,
        request_id: uuid(2)?,
        target: EnvironmentRequestTarget {
            session_id: uuid(3)?,
            generation: uuid(4)?,
        },
        timestamp_unix_ms: integer(5)?,
        actor: bounded_sql_text(row, 6, 2 * uuid::fmt::Hyphenated::LENGTH + 4)?,
        state,
        composition: EnvironmentApplicationComposition {
            boot: reference(8)?,
            live_policy: reference(10)?,
            desired: reference(12)?,
            corporate_policy_id: bounded_sql_text(row, 14, capsem_proto::APPROVAL_CONTEXT_BYTES)?,
            pending_restart,
        },
        failure_reason: bounded_optional_sql_text(row, 16, capsem_proto::APPROVAL_CONTEXT_BYTES)?,
    };
    if record.cursor <= 0
        || record.version <= 0
        || record.timestamp_unix_ms < 0
        || record.composition.corporate_policy_id.trim().is_empty()
        || record.composition.corporate_policy_id.contains('\0')
        || (state == EnvironmentApplicationState::Failed) != record.failure_reason.is_some()
        || record
            .failure_reason
            .as_ref()
            .is_some_and(|reason| reason.trim().is_empty() || reason.contains('\0'))
        || (record.composition.boot == record.composition.desired
            && record.composition.live_policy == record.composition.desired
            && pending_restart)
    {
        return Err("invalid environment application metadata".into());
    }
    Ok(record)
}
