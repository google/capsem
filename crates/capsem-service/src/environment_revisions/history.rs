//! Operator inspection of committed ledger history, including partial records.
use super::*;
use capsem_api::{
    environments::{
        EnvironmentRevisionDetailResponse, EnvironmentRevisionListResponse, EnvironmentRevisionReference,
        EnvironmentRevisionSummary,
    },
    requests::RequestReviewAvailability,
};
use capsem_logger::db::EnvironmentHistoryPayload;

impl EnvironmentRevisions {
    pub async fn list_history(
        &self,
        id: String,
        before: Option<i64>,
        limit: u16,
    ) -> Result<EnvironmentRevisionListResponse, EnvironmentFileError> {
        let mut records = self
            .ledger
            .environment_revisions(EnvironmentRevisionQuery::History {
                environment_id: id,
                before_id: before,
                limit: limit.saturating_add(1),
            })
            .await
            .map_err(|_| EnvironmentFileError::LedgerFailure)?;
        let more = records.len() > usize::from(limit);
        records.truncate(usize::from(limit));
        let next_cursor = if more {
            records.last().map(|record| record.cursor)
        } else {
            None
        };
        Ok(EnvironmentRevisionListResponse {
            revisions: records.into_iter().map(summary).collect(),
            next_cursor,
        })
    }

    pub async fn revision_detail(
        &self,
        id: &str,
        revision: &str,
    ) -> Result<Option<EnvironmentRevisionDetailResponse>, EnvironmentFileError> {
        let Some(history) = self
            .ledger
            .environment_revision_history(id, revision)
            .await
            .map_err(|_| EnvironmentFileError::LedgerFailure)?
        else {
            return Ok(None);
        };
        if history.record.state != EnvironmentRevisionState::Committed {
            return Ok(None);
        }
        let (availability, canonical_toml, partial_specification, purged_variables) = match history.payload {
            EnvironmentHistoryPayload::Complete { value } => {
                let document = EnvironmentDocument {
                    identity: identity(id, revision, history.record.fork_parent.as_ref()),
                    specification: *value,
                };
                let toml = tokio::task::spawn_blocking(move || document.to_toml())
                    .await
                    .map_err(|_| EnvironmentFileError::WorkerFailed)?
                    .map_err(|_| EnvironmentFileError::InvalidDocument)?;
                (RequestReviewAvailability::Complete, Some(toml), None, Vec::new())
            }
            EnvironmentHistoryPayload::Purged {
                value,
                purged_variables,
            } => (RequestReviewAvailability::Purged, None, Some(value), purged_variables),
            EnvironmentHistoryPayload::Unavailable { purged_variables } => {
                (RequestReviewAvailability::Unavailable, None, None, purged_variables)
            }
        };
        Ok(Some(EnvironmentRevisionDetailResponse {
            summary: summary(history.record),
            availability,
            canonical_toml,
            partial_specification,
            purged_variables: purged_variables
                .into_iter()
                .map(|marker| marker.variable_name)
                .collect(),
        }))
    }
}

fn summary(record: EnvironmentRevisionRecord) -> EnvironmentRevisionSummary {
    EnvironmentRevisionSummary {
        environment: EnvironmentRevisionReference {
            environment_id: record.environment_id,
            revision_id: record.revision_id,
        },
        previous_revision_id: record.previous_revision_id,
        fork_parent: record.fork_parent.map(|parent| EnvironmentRevisionReference {
            environment_id: parent.environment_id,
            revision_id: parent.revision_id,
        }),
        timestamp_unix_ms: record.timestamp_unix_ms,
        actor: record.actor,
        reason: record.reason,
        request_id: record.request_id,
        replayable: record.replayable,
    }
}
