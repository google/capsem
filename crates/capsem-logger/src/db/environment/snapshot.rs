//! Required snapshots reconstruct from their immutable archive, never current files.
use super::*;
use crate::db::bodies::{self, CapturedBodies, IndexRow};
use capsem_archive::{ArchiveCodecs, BodyLogReader};
use capsem_config::environment::EnvironmentSpecification;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentRevisionSnapshot {
    pub record: EnvironmentRevisionRecord,
    pub specification: Box<EnvironmentSpecification>,
}

pub(in crate::db) struct CapturedRevision {
    reader: BodyLogReader,
    record: EnvironmentRevisionRecord,
    index: IndexRow,
}

impl DbHandle {
    pub async fn environment_revision_snapshot(
        &self,
        environment_id: &str,
        revision_id: &str,
    ) -> DbResult<Option<EnvironmentRevisionSnapshot>> {
        let query = EnvironmentRevisionQuery::Get {
            environment_id: environment_id.into(),
            revision_id: revision_id.into(),
        };
        query.validate()?;
        let (reply, receiver) = tokio::sync::oneshot::channel();
        self.send_reader(ReadRequest::EnvironmentSnapshot { query, reply })
            .map_err(|_| "environment snapshot reader worker closed")?;
        let captured = receiver
            .await
            .map_err(|_| "environment snapshot reader dropped reply")??;
        let Some(captured) = self.take_observed(captured) else {
            return Ok(None);
        };
        let handle = self.clone();
        tokio::task::spawn_blocking(move || {
            let CapturedRevision { reader, record, index } = captured;
            let body = handle
                .read_archived_blocking(CapturedBodies {
                    reader,
                    rows: vec![index],
                })?
                .pop()
                .ok_or("required environment snapshot payload is missing")?;
            let specification: EnvironmentSpecification = rmp_serde::from_slice(&body.bytes)
                .map_err(|_| "invalid archived environment specification encoding")?;
            specification
                .validate()
                .map_err(|_| "invalid archived environment specification")?;
            Ok(Some(EnvironmentRevisionSnapshot {
                record,
                specification: Box::new(specification),
            }))
        })
        .await
        .map_err(|_| "environment snapshot reconstruction worker failed")?
    }
}

pub(in crate::db) fn capture(
    reader: &DbReader,
    path: &std::path::Path,
    query: &EnvironmentRevisionQuery,
    codecs: &ArchiveCodecs,
) -> DbResult<Option<CapturedRevision>> {
    if !matches!(query, EnvironmentRevisionQuery::Get { .. }) {
        return Err("environment snapshot requires exact revision identity".into());
    }
    let (reader, selected) = bodies::with_archive_snapshot(reader, path, codecs, |connection, deadline| {
        deadline.check("environment revision capture")?;
        let Some(record) = query_rows(connection, query)
            .map_err(|error| format!("environment snapshot metadata: {error}"))?
            .pop()
        else {
            return Ok(None);
        };
        if !record.replayable {
            return Err("environment revision is non-replayable after variable history was purged".into());
        }
        let index =
            bodies::required_configuration_index(connection, &record.payload_event_id, "environment_revisions")?;
        Ok(Some((record, index)))
    })?;
    Ok(selected.map(|(record, index)| CapturedRevision { reader, record, index }))
}
