//! Exact durable pointers; callers never infer current state from history order.
use super::*;
use crate::events::environment::EnvironmentSandboxReferenceKind;

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EnvironmentReferenceQuery {
    CurrentEnvironment { environment_id: String },
    Sandbox { session_id: String, generation: String },
}

impl EnvironmentReferenceQuery {
    pub(crate) fn validate(&self) -> DbResult<()> {
        let valid = match self {
            Self::CurrentEnvironment { environment_id } => valid_id(environment_id),
            Self::Sandbox { session_id, generation } => valid_id(session_id) && valid_id(generation),
        };
        if valid {
            Ok(())
        } else {
            Err("invalid environment reference query identity".into())
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EnvironmentReferenceRecord {
    CurrentEnvironment {
        revision: EnvironmentRevisionParent,
    },
    Sandbox {
        dimension: EnvironmentSandboxReferenceKind,
        revision: EnvironmentRevisionParent,
    },
}

impl DbHandle {
    /// One exact owner, with the independent sandbox dimensions read together.
    /// These stored facts confer no mutation or runtime authority.
    pub async fn environment_references(
        &self,
        query: EnvironmentReferenceQuery,
    ) -> DbResult<Vec<EnvironmentReferenceRecord>> {
        query.validate()?;
        let (reply, receiver) = tokio::sync::oneshot::channel();
        self.send_reader(ReadRequest::EnvironmentReferences { query, reply })
            .map_err(|_| "environment reference reader worker closed")?;
        receiver
            .await
            .map_err(|_| "environment reference reader dropped reply")?
            .map(|observed| self.take_observed(observed))
    }
}

pub(in crate::db) fn execute(
    reader: &DbReader,
    query: &EnvironmentReferenceQuery,
) -> DbResult<Vec<EnvironmentReferenceRecord>> {
    let started = Instant::now();
    let _deadline = reader.sql_deadline(started + super::bodies::INTERACTIVE_CAPTURE_DEADLINE);
    let result = query_rows(reader.connection(), query);
    record_metadata_query("environment_reference_execute", started, &result);
    result
}

fn query_rows(
    connection: &rusqlite::Connection,
    query: &EnvironmentReferenceQuery,
) -> DbResult<Vec<EnvironmentReferenceRecord>> {
    let (filter, owner_id, generation, maximum) = match query {
        EnvironmentReferenceQuery::CurrentEnvironment { environment_id } => (
            "p.owner_kind='current_environment' AND p.owner_id=?1 AND ?2=''",
            environment_id, "", 1,
        ),
        EnvironmentReferenceQuery::Sandbox { session_id, generation } => (
            "p.owner_kind IN ('sandbox_boot','sandbox_live_policy','sandbox_desired') AND p.owner_id=?1 AND p.owner_generation=?2",
            session_id, generation.as_str(), 3,
        ),
    };
    let mut statement = connection
        .prepare_cached(&format!(
            "SELECT p.owner_kind,p.owner_generation,p.environment_id,p.revision_id,r.state
         FROM environment_revision_references p LEFT JOIN environment_revisions r
         ON r.environment_id=p.environment_id AND r.revision_id=p.revision_id
         WHERE {filter} ORDER BY p.owner_kind LIMIT 4"
        ))
        .map_err(|error| format!("prepare environment reference query: {error}"))?;
    let mut rows = statement
        .query(rusqlite::params![owner_id, generation])
        .map_err(|error| format!("execute environment reference query: {error}"))?;
    let mut records = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|error| format!("read environment reference: {error}"))?
    {
        use crate::db::bodies::bounded_sql_text;
        if records.len() == maximum {
            return Err("duplicate environment reference dimensions".into());
        }
        let kind = bounded_sql_text(row, 0, 32)?;
        let stored_generation = bounded_sql_text(row, 1, uuid::fmt::Hyphenated::LENGTH)?;
        let revision = EnvironmentRevisionParent {
            environment_id: bounded_sql_text(row, 2, uuid::fmt::Hyphenated::LENGTH)?,
            revision_id: bounded_sql_text(row, 3, uuid::fmt::Hyphenated::LENGTH)?,
        };
        if !valid_id(&revision.environment_id)
            || !valid_id(&revision.revision_id)
            || bounded_sql_text(row, 4, 32)? != "committed"
        {
            return Err("invalid environment reference target".into());
        }
        let record = match query {
            EnvironmentReferenceQuery::CurrentEnvironment { environment_id } => {
                if kind != "current_environment"
                    || !stored_generation.is_empty()
                    || revision.environment_id != *environment_id
                {
                    return Err("invalid canonical environment reference owner".into());
                }
                EnvironmentReferenceRecord::CurrentEnvironment { revision }
            }
            EnvironmentReferenceQuery::Sandbox { generation, .. } => {
                if stored_generation != *generation {
                    return Err("invalid sandbox reference generation".into());
                }
                let dimension = match kind.as_str() {
                    "sandbox_boot" => EnvironmentSandboxReferenceKind::Boot,
                    "sandbox_live_policy" => EnvironmentSandboxReferenceKind::LivePolicy,
                    "sandbox_desired" => EnvironmentSandboxReferenceKind::Desired,
                    _ => return Err("invalid sandbox reference dimension".into()),
                };
                EnvironmentReferenceRecord::Sandbox { dimension, revision }
            }
        };
        records.push(record);
    }
    Ok(records)
}
