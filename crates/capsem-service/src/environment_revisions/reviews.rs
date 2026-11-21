//! Metadata pages and exact, purge-aware review content read from the ledger.
use super::{
    mutations::{now, view},
    EnvironmentMutationError as Error, EnvironmentRevisionParent, EnvironmentRevisions,
};
use capsem_api::{
    environments::{EnvironmentRevisionReference, EnvironmentSandboxTarget},
    requests::*,
};
use capsem_logger::{
    db::{EnvironmentHistoryPayload, EnvironmentRequestQuery, EnvironmentRequestRecord},
    events::environment::request::{EnvironmentRequestKind, EnvironmentRequestPayload, EnvironmentRequestTarget},
};

impl EnvironmentRevisions {
    pub async fn list_requests(&self, query: RequestListQuery) -> Result<RequestListResponse, Error> {
        let maximum = capsem_proto::APPROVAL_PENDING_TOTAL as u16;
        let limit = query.limit.unwrap_or(maximum);
        let after_id = query.after.unwrap_or_default();
        if after_id < 0 || limit == 0 || limit > maximum {
            return Err(Error::InvalidIntent);
        }
        let records = self
            .ledger
            .environment_requests(EnvironmentRequestQuery::Page {
                environment_id: None,
                requester: None,
                pending_only: query.pending_only.unwrap_or_default(),
                after_id,
                limit,
            })
            .await
            .map_err(|_| Error::LedgerFailure)?;
        let at = now()?;
        let next_cursor = (records.len() == usize::from(limit))
            .then(|| records.last().map(|record| record.cursor))
            .flatten();
        let requests = records
            .into_iter()
            .map(|record| summary(record, at))
            .filter(|item| query.pending_only != Some(true) || item.decision == RequestState::Pending)
            .collect();
        Ok(RequestListResponse { requests, next_cursor })
    }

    pub async fn review_request(&self, id: &str) -> Result<RequestDetailResponse, Error> {
        let history = self
            .ledger
            .environment_request_history(id)
            .await
            .map_err(|_| Error::LedgerFailure)?
            .ok_or(Error::NotFound)?;
        let observed_revision = history.record.payload_event_id.clone();
        let disposition = if let Some(invalidation) = &history.record.invalidation {
            Some(RequestDecisionDetails {
                actor: invalidation.actor.clone(),
                reason: invalidation.reason.clone(),
                timestamp_unix_ms: invalidation.timestamp_unix_ms,
            })
        } else {
            history.record.decision.as_ref().map(|decision| RequestDecisionDetails {
                actor: decision.actor.clone(),
                reason: decision.reason.clone(),
                timestamp_unix_ms: decision.timestamp_unix_ms,
            })
        };
        let summary = summary(history.record, now()?);
        let base = match &summary.base {
            Some(base) => Some(self.revision_review(base).await?),
            None => None,
        };
        let (availability, payload, purged_variables) = content(history.payload);
        let mut proposed = EnvironmentReviewContent {
            availability,
            specification: None,
            purged_variables,
        };
        let mut targets = Vec::new();
        if let Some(payload) = payload {
            match *payload {
                EnvironmentRequestPayload::Archive { .. } => {}
                EnvironmentRequestPayload::Change { specification }
                | EnvironmentRequestPayload::ImageUpdate { specification } => {
                    proposed.specification = Some(specification);
                }
                EnvironmentRequestPayload::Promotion { source } => {
                    proposed = self.revision_review(&reference(source)).await?;
                }
                EnvironmentRequestPayload::Apply {
                    revision,
                    targets: intended,
                } => {
                    proposed = self.revision_review(&reference(revision)).await?;
                    targets = intended.into_iter().map(target).collect();
                }
            }
        }
        Ok(RequestDetailResponse {
            summary,
            observed_revision,
            disposition,
            base,
            proposed,
            targets,
        })
    }

    async fn revision_review(
        &self,
        reference: &EnvironmentRevisionReference,
    ) -> Result<EnvironmentReviewContent, Error> {
        let history = self
            .ledger
            .environment_revision_history(&reference.environment_id, &reference.revision_id)
            .await
            .map_err(|_| Error::LedgerFailure)?;
        let payload =
            history
                .map(|history| history.payload)
                .unwrap_or_else(|| EnvironmentHistoryPayload::Unavailable {
                    purged_variables: Vec::new(),
                });
        let (availability, specification, purged_variables) = content(payload);
        Ok(EnvironmentReviewContent {
            availability,
            specification,
            purged_variables,
        })
    }
}

fn summary(record: EnvironmentRequestRecord, at: i64) -> RequestSummary {
    let observed = view(record.clone());
    let kind = match record.kind {
        EnvironmentRequestKind::Change => RequestKind::EnvironmentChange,
        EnvironmentRequestKind::Archive => RequestKind::EnvironmentArchive,
        EnvironmentRequestKind::ImageUpdate => RequestKind::EnvironmentImageUpdate,
        EnvironmentRequestKind::Promotion => RequestKind::EnvironmentPromotion,
        EnvironmentRequestKind::Apply => RequestKind::EnvironmentApply,
    };
    let base = record.base_revision_id.map(|revision_id| EnvironmentRevisionReference {
        environment_id: record.environment_id.clone(),
        revision_id,
    });
    // Report an elapsed review window without changing the durable decision on a read.
    let decision = if observed.decision == RequestState::Pending && at >= record.expires_at_unix_ms {
        RequestState::Expired
    } else {
        observed.decision
    };
    RequestSummary {
        request_id: record.request_id,
        kind,
        environment_id: record.environment_id,
        base,
        source: record.source.map(reference),
        requester: record.requester.map(target),
        actor: record.actor,
        reason: record.reason,
        timestamp_unix_ms: record.timestamp_unix_ms,
        expires_at_unix_ms: record.expires_at_unix_ms,
        decision,
        application: observed.application,
        result: observed.result,
    }
}

fn reference(source: EnvironmentRevisionParent) -> EnvironmentRevisionReference {
    EnvironmentRevisionReference {
        environment_id: source.environment_id,
        revision_id: source.revision_id,
    }
}

fn target(value: EnvironmentRequestTarget) -> EnvironmentSandboxTarget {
    EnvironmentSandboxTarget {
        session_id: value.session_id,
        generation: value.generation,
    }
}

fn content<T>(payload: EnvironmentHistoryPayload<T>) -> (RequestReviewAvailability, Option<Box<T>>, Vec<String>) {
    use EnvironmentHistoryPayload::{Complete, Purged, Unavailable};
    let (availability, value, purges) = match payload {
        Complete { value } => return (RequestReviewAvailability::Complete, Some(value), Vec::new()),
        Purged {
            value,
            purged_variables,
        } => (RequestReviewAvailability::Purged, Some(value), purged_variables),
        Unavailable { purged_variables } => (RequestReviewAvailability::Unavailable, None, purged_variables),
    };
    (
        availability,
        value,
        purges.into_iter().map(|marker| marker.variable_name).collect(),
    )
}
