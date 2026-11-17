//! Required snapshots reconstruct from their immutable archive, never current files.
use super::history::{self, EnvironmentHistoryPayload, EnvironmentVariablePurgeMarker, SnapshotMode};
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentRevisionHistory {
    pub record: EnvironmentRevisionRecord,
    pub payload: EnvironmentHistoryPayload<EnvironmentSpecification>,
}

pub(in crate::db) struct CapturedRevision {
    reader: BodyLogReader,
    record: EnvironmentRevisionRecord,
    index: Option<IndexRow>,
    purges: Vec<EnvironmentVariablePurgeMarker>,
}

impl DbHandle {
    pub async fn environment_revision_snapshot(
        &self,
        environment_id: &str,
        revision_id: &str,
    ) -> DbResult<Option<EnvironmentRevisionSnapshot>> {
        let Some(history) = self
            .revision_history(environment_id, revision_id, SnapshotMode::Replay)
            .await?
        else {
            return Ok(None);
        };
        let EnvironmentHistoryPayload::Complete { value } = history.payload else {
            return Err("environment revision is non-replayable".into());
        };
        Ok(Some(EnvironmentRevisionSnapshot {
            record: history.record,
            specification: value,
        }))
    }

    /// Historical partial values carry explicit purge markers and cannot be replayed.
    pub async fn environment_revision_history(
        &self,
        environment_id: &str,
        revision_id: &str,
    ) -> DbResult<Option<EnvironmentRevisionHistory>> {
        self.revision_history(environment_id, revision_id, SnapshotMode::History)
            .await
    }

    async fn revision_history(
        &self,
        environment_id: &str,
        revision_id: &str,
        mode: SnapshotMode,
    ) -> DbResult<Option<EnvironmentRevisionHistory>> {
        let query = EnvironmentRevisionQuery::Get {
            environment_id: environment_id.into(),
            revision_id: revision_id.into(),
        };
        query.validate()?;
        let (reply, receiver) = tokio::sync::oneshot::channel();
        self.send_reader(ReadRequest::EnvironmentSnapshot { query, mode, reply })
            .map_err(|_| "environment snapshot reader worker closed")?;
        let captured = receiver
            .await
            .map_err(|_| "environment snapshot reader dropped reply")??;
        let Some(captured) = self.take_observed(captured) else {
            return Ok(None);
        };
        let handle = self.clone();
        tokio::task::spawn_blocking(move || {
            let CapturedRevision {
                reader,
                record,
                index,
                purges,
            } = captured;
            let specification = if let Some(index) = index {
                let body = bodies::read_archived_blocking(
                    &handle,
                    CapturedBodies {
                        reader,
                        rows: vec![index],
                    },
                )?
                .pop()
                .ok_or("required environment snapshot payload is missing")?;
                let specification: EnvironmentSpecification = rmp_serde::from_slice(&body.bytes)
                    .map_err(|_| "invalid archived environment specification encoding")?;
                specification
                    .validate()
                    .map_err(|_| "invalid archived environment specification")?;
                if purges
                    .iter()
                    .any(|marker| specification.variables.contains_key(&marker.variable_name))
                {
                    return Err("archived environment values contradict published purge markers".into());
                }
                Some(Box::new(specification))
            } else {
                None
            };
            let payload = history::payload(record.replayable, specification, purges)?;
            Ok(Some(EnvironmentRevisionHistory { record, payload }))
        })
        .await
        .map_err(|_| "environment snapshot reconstruction worker failed")?
    }
}

pub(in crate::db) fn capture(
    reader: &DbReader,
    path: &std::path::Path,
    query: &EnvironmentRevisionQuery,
    mode: SnapshotMode,
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
        let purges = history::revision_purges(connection, &record.revision_id)?;
        let index = history::index(
            connection,
            &record.payload_event_id,
            "environment_revisions",
            record.replayable,
            mode,
            &purges,
        )?;
        Ok(Some((record, index, purges)))
    })?;
    Ok(selected.map(|(record, index, purges)| CapturedRevision {
        reader,
        record,
        index,
        purges,
    }))
}
