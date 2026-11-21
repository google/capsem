//! Resolve an observed immutable request and publish its approved fork.
use super::*;
use capsem_api::{
    requests::{RequestDecision, RequestResolveRequest},
    CallerActor,
};
use std::future::Future;

pub struct ReviewedEnvironmentChange {
    pub base: EnvironmentRevisionParent,
    pub target: EnvironmentRequestTarget,
    pub specification: Box<EnvironmentSpecification>,
}

pub struct ApprovedEnvironmentReview<T> {
    pub request_id: String,
    pub result: EnvironmentRevisionParent,
    pub runtime: T,
}

pub struct ResolvedEnvironmentReview<T> {
    pub response: EnvironmentMutationResponse,
    pub approved: Option<ApprovedEnvironmentReview<T>>,
}

impl EnvironmentMutationCoordinator {
    /// The authenticated service supplies the caller and checks current runtime admission.
    /// An exact retry recovers the receipt without repeating publication.
    pub async fn resolve_proposal<T, F>(
        &self,
        id: String,
        intent: RequestResolveRequest,
        caller: &CallerActor,
        authorize_cancel: impl Fn(&EnvironmentRequestTarget) -> bool,
        verify_approval: impl FnOnce(ReviewedEnvironmentChange) -> F,
    ) -> Result<ResolvedEnvironmentReview<T>, EnvironmentMutationError>
    where
        F: Future<Output = Result<T, EnvironmentMutationError>>,
    {
        use EnvironmentMutationError as Error;

        capsem_api::validate_managed_request_id(&id).map_err(|_| Error::InvalidIntent)?;
        intent.validate().map_err(|_| Error::InvalidIntent)?;
        let lock = self
            .revisions
            .lock_for(&format!("request:{id}"))
            .map_err(Error::Publication)?;
        let _guard = lock.lock().await;
        let record = self.find_request(id.clone()).await?.ok_or(Error::NotFound)?;
        let decision = match intent.decision {
            RequestDecision::Approve => EnvironmentRequestDecisionKind::Approve,
            RequestDecision::Deny => EnvironmentRequestDecisionKind::Deny,
            RequestDecision::Cancel => EnvironmentRequestDecisionKind::Cancel,
        };
        let caller_id = caller.ledger_id();
        match (caller, decision) {
            (CallerActor::Operator, EnvironmentRequestDecisionKind::Approve | EnvironmentRequestDecisionKind::Deny) => {
            }
            (CallerActor::Operator, EnvironmentRequestDecisionKind::Cancel) if record.actor == caller_id => {}
            (CallerActor::DelegatedAgent { .. }, EnvironmentRequestDecisionKind::Cancel)
                if record.actor == caller_id && record.requester.as_ref().is_some_and(authorize_cancel) => {}
            _ => return Err(Error::Refused),
        }
        if record.payload_event_id != intent.observed_revision {
            return Err(Error::Conflict);
        }
        if record.decision.as_ref().is_some_and(|saved| {
            saved.decision == decision && saved.actor == caller_id && saved.reason == intent.reason
        }) || matches!(
            record.state,
            EnvironmentRequestState::Expired | EnvironmentRequestState::Stale
        ) {
            return Ok(ResolvedEnvironmentReview {
                response: view(record),
                approved: None,
            });
        }
        if record.state != EnvironmentRequestState::Pending {
            return Err(Error::Conflict);
        }
        let at = now()?;
        if at >= record.expires_at_unix_ms {
            return self
                .invalidate_review(id, EnvironmentRequestInvalidationKind::Expired, at)
                .await;
        }

        let (decision_at, approved) = if decision == EnvironmentRequestDecisionKind::Approve {
            let change = self.reviewed_change(&record).await?;
            let base = change.base.clone();
            let frozen = change.specification.clone();
            let verified = verify_approval(change).await;
            let at = now()?;
            if at >= record.expires_at_unix_ms {
                return self
                    .invalidate_review(id, EnvironmentRequestInvalidationKind::Expired, at)
                    .await;
            }
            let runtime = match verified {
                Ok(runtime) => runtime,
                Err(Error::Conflict) => {
                    return self
                        .invalidate_review(id, EnvironmentRequestInvalidationKind::Stale, at)
                        .await;
                }
                Err(error) => return Err(error),
            };
            (at, Some((runtime, base, frozen)))
        } else {
            (at, None)
        };
        self.write(WriteOp::EnvironmentRequestDecision(EnvironmentRequestDecision {
            request_id: id.clone(),
            timestamp_unix_ms: decision_at,
            actor: caller_id,
            decision,
            reason: intent.reason.clone(),
        }))
        .await?;

        let approved = if let Some((runtime, base, specification)) = approved {
            let revision = self
                .revisions
                .commit(
                    PreparedEnvironmentRevision {
                        environment_id: capsem_crypto::uuid_v4().to_string(),
                        revision_id: capsem_crypto::uuid_v4().to_string(),
                        previous_revision_id: None,
                        fork_parent: Some(base),
                        timestamp_unix_ms: decision_at,
                        actor: "operator".into(),
                        reason: intent.reason,
                        request_id: Some(id.clone()),
                        payload_event_id: event_id(),
                        specification,
                    },
                    decision_at,
                )
                .wait()
                .await
                .map_err(Error::Publication)?;
            Some(ApprovedEnvironmentReview {
                request_id: id.clone(),
                result: EnvironmentRevisionParent {
                    environment_id: revision.environment_id,
                    revision_id: revision.revision_id,
                },
                runtime,
            })
        } else {
            None
        };
        Ok(ResolvedEnvironmentReview {
            response: view(self.read_request(id).await?),
            approved,
        })
    }

    async fn reviewed_change(
        &self,
        record: &EnvironmentRequestRecord,
    ) -> Result<ReviewedEnvironmentChange, EnvironmentMutationError> {
        use EnvironmentMutationError as Error;
        let snapshot = self
            .revisions
            .ledger
            .environment_request_history(&record.request_id)
            .await
            .map_err(|_| Error::LedgerFailure)?
            .ok_or(Error::Conflict)?;
        let capsem_logger::db::EnvironmentHistoryPayload::Complete { value } = snapshot.payload else {
            return Err(Error::Conflict);
        };
        let EnvironmentRequestPayload::Change { specification } = *value else {
            return Err(Error::Conflict);
        };
        Ok(ReviewedEnvironmentChange {
            base: EnvironmentRevisionParent {
                environment_id: record.environment_id.clone(),
                revision_id: record.base_revision_id.clone().ok_or(Error::Conflict)?,
            },
            target: record.requester.clone().ok_or(Error::Conflict)?,
            specification,
        })
    }

    async fn invalidate_review<T>(
        &self,
        id: String,
        kind: EnvironmentRequestInvalidationKind,
        at: i64,
    ) -> Result<ResolvedEnvironmentReview<T>, EnvironmentMutationError> {
        let reason = match kind {
            EnvironmentRequestInvalidationKind::Expired => "review deadline expired before a decision",
            EnvironmentRequestInvalidationKind::Stale => "original sandbox generation or reviewed base changed",
        };
        self.write(WriteOp::EnvironmentRequestInvalidation(
            EnvironmentRequestInvalidation {
                request_id: id.clone(),
                timestamp_unix_ms: at,
                actor: "system".into(),
                kind,
                reason: reason.into(),
            },
        ))
        .await?;
        Ok(ResolvedEnvironmentReview {
            response: view(self.read_request(id).await?),
            approved: None,
        })
    }
}
