//! Trusted operator coordination. Authenticated routes admit the caller before
//! invoking this owner; no request-body actor or resolution proof is accepted.
use super::*;
use capsem_api::{
    environments::*,
    requests::{RequestApplicationState, RequestState},
};
use capsem_core::environment::{
    resolve_environment, EnvironmentPinResolver, EnvironmentResolutionError, EnvironmentResolutionMode,
};
use capsem_logger::{
    db::{EnvironmentRequestQuery, EnvironmentRequestRecord},
    events::environment::request::*,
};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvironmentCreateError {
    InvalidIntent,
    Conflict,
    Resolution(EnvironmentResolutionError),
    LedgerFailure,
    Publication(EnvironmentFileError),
    WorkerFailed,
}
impl std::fmt::Display for EnvironmentCreateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidIntent => "invalid operator environment creation intent",
            Self::Conflict => "request identity already has different or non-replayable intent",
            Self::Resolution(_) => "environment source resolution failed",
            Self::LedgerFailure => "environment request durability is unconfirmed",
            Self::Publication(_) => "environment publication is unconfirmed; reconcile the request",
            Self::WorkerFailed => "environment creation worker failed",
        })
    }
}
impl std::error::Error for EnvironmentCreateError {}

pub struct EnvironmentCreateJob {
    receiver: oneshot::Receiver<Result<EnvironmentMutationResponse, EnvironmentCreateError>>,
}
impl EnvironmentCreateJob {
    pub async fn wait(self) -> Result<EnvironmentMutationResponse, EnvironmentCreateError> {
        self.receiver.await.map_err(|_| EnvironmentCreateError::WorkerFailed)?
    }
}

pub struct EnvironmentCreateCoordinator {
    revisions: Arc<EnvironmentRevisions>,
}
impl EnvironmentCreateCoordinator {
    pub fn new(revisions: Arc<EnvironmentRevisions>) -> Self {
        Self { revisions }
    }

    /// Dropping the HTTP response or this job cannot cancel accepted work.
    /// The source capability is selected by the host, never deserialized.
    pub fn submit<R>(
        self: &Arc<Self>,
        request: EnvironmentCreateRequest,
        sources: R,
    ) -> Result<EnvironmentCreateJob, EnvironmentCreateError>
    where
        R: EnvironmentPinResolver + Send + 'static,
    {
        request.validate().map_err(|_| EnvironmentCreateError::InvalidIntent)?;
        if request.requesting_sandbox.is_some() {
            return Err(EnvironmentCreateError::InvalidIntent);
        }
        let owner = self.clone();
        let (sender, receiver) = oneshot::channel();
        tokio::spawn(async move {
            let result = owner.create(request, sources).await;
            drop(owner);
            let _ = sender.send(result);
        });
        Ok(EnvironmentCreateJob { receiver })
    }

    async fn create(
        &self,
        request: EnvironmentCreateRequest,
        sources: impl EnvironmentPinResolver,
    ) -> Result<EnvironmentMutationResponse, EnvironmentCreateError> {
        // A request namespace shares the existing owner's keyed lock lifetime,
        // without holding a policy/environment lock during network resolution.
        let lock = self
            .revisions
            .lock_for(&format!("request:{}", request.request_id))
            .map_err(EnvironmentCreateError::Publication)?;
        let _guard = lock.lock().await;
        if let Some(saved) = self.reconcile(&request).await? {
            return Ok(saved);
        }
        let specification = resolve_environment(request.specification, None, EnvironmentResolutionMode::Edit, &sources)
            .await
            .map_err(EnvironmentCreateError::Resolution)?;
        let at = now()?;
        let environment_id = capsem_crypto::uuid_v4().to_string();
        self.write(WriteOp::PrepareEnvironmentRequest(PreparedEnvironmentRequest {
            request_id: request.request_id.clone(),
            environment_id: environment_id.clone(),
            base_revision_id: None,
            requester: None,
            timestamp_unix_ms: at,
            // Operator intent is approved immediately, never held for human waiting.
            expires_at_unix_ms: at
                .checked_add(
                    i64::try_from(capsem_proto::APPROVAL_TIMEOUT_SECONDS)
                        .ok()
                        .and_then(|seconds| seconds.checked_mul(1000))
                        .ok_or(EnvironmentCreateError::WorkerFailed)?,
                )
                .ok_or(EnvironmentCreateError::WorkerFailed)?,
            actor: "operator".into(),
            reason: request.reason.clone(),
            payload_event_id: event_id(),
            payload: Box::new(EnvironmentRequestPayload::Change {
                specification: Box::new(specification),
            }),
        }))
        .await?;
        // Approval and publication consume the durable frozen request, not a
        // second resolution or a cloned mutable request payload.
        let saved = self
            .revisions
            .ledger
            .environment_request_snapshot(&request.request_id)
            .await
            .map_err(|_| EnvironmentCreateError::LedgerFailure)?
            .ok_or(EnvironmentCreateError::LedgerFailure)?;
        let EnvironmentRequestPayload::Change { specification } = *saved.payload else {
            return Err(EnvironmentCreateError::Conflict);
        };
        self.write(WriteOp::EnvironmentRequestDecision(EnvironmentRequestDecision {
            request_id: request.request_id.clone(),
            timestamp_unix_ms: now()?,
            actor: "operator".into(),
            decision: EnvironmentRequestDecisionKind::Approve,
            reason: request.reason.clone(),
        }))
        .await?;
        let at = now()?;
        self.revisions
            .commit(
                PreparedEnvironmentRevision {
                    environment_id,
                    revision_id: capsem_crypto::uuid_v4().to_string(),
                    previous_revision_id: None,
                    fork_parent: None,
                    timestamp_unix_ms: at,
                    actor: "operator".into(),
                    reason: request.reason,
                    request_id: Some(request.request_id.clone()),
                    payload_event_id: event_id(),
                    specification,
                },
                at,
            )
            .wait()
            .await
            .map_err(EnvironmentCreateError::Publication)?;
        let record = self
            .revisions
            .ledger
            .environment_requests(EnvironmentRequestQuery::Get {
                request_id: request.request_id,
            })
            .await
            .map_err(|_| EnvironmentCreateError::LedgerFailure)?
            .pop()
            .ok_or(EnvironmentCreateError::LedgerFailure)?;
        Ok(view(record))
    }

    async fn reconcile(
        &self,
        request: &EnvironmentCreateRequest,
    ) -> Result<Option<EnvironmentMutationResponse>, EnvironmentCreateError> {
        let Some(saved) = self
            .revisions
            .ledger
            .environment_request_history(&request.request_id)
            .await
            .map_err(|_| EnvironmentCreateError::LedgerFailure)?
        else {
            return Ok(None);
        };
        let capsem_logger::db::EnvironmentHistoryPayload::Complete { value } = saved.payload else {
            return Err(EnvironmentCreateError::Conflict);
        };
        let mut record = saved.record;
        let EnvironmentRequestPayload::Change { specification } = *value else {
            return Err(EnvironmentCreateError::Conflict);
        };
        if record.actor != "operator"
            || record.requester.is_some()
            || record.base_revision_id.is_some()
            || record.reason != request.reason
            || specification.into_draft() != request.specification
        {
            return Err(EnvironmentCreateError::Conflict);
        }
        let at = now()?;
        if record.state == EnvironmentRequestState::Pending && at >= record.expires_at_unix_ms {
            self.write(WriteOp::EnvironmentRequestInvalidation(
                EnvironmentRequestInvalidation {
                    request_id: request.request_id.clone(),
                    timestamp_unix_ms: at,
                    actor: "operator".into(),
                    kind: EnvironmentRequestInvalidationKind::Expired,
                    reason: "review deadline expired before an operator decision".into(),
                },
            ))
            .await?;
            record = self
                .revisions
                .ledger
                .environment_requests(EnvironmentRequestQuery::Get {
                    request_id: request.request_id.clone(),
                })
                .await
                .map_err(|_| EnvironmentCreateError::LedgerFailure)?
                .pop()
                .ok_or(EnvironmentCreateError::LedgerFailure)?;
        }
        // Pending, interrupted or stale requests remain visible. Reconciliation
        // never casts another vote, creates a revision, or retries application.
        Ok(Some(view(record)))
    }

    async fn write(&self, operation: WriteOp) -> Result<(), EnvironmentCreateError> {
        self.revisions
            .ledger
            .write(operation)
            .await
            .map_err(|_| EnvironmentCreateError::LedgerFailure)?;
        self.revisions
            .ledger
            .flush()
            .await
            .map_err(|_| EnvironmentCreateError::LedgerFailure)
    }
}
fn now() -> Result<i64, EnvironmentCreateError> {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| EnvironmentCreateError::WorkerFailed)?
            .as_millis(),
    )
    .map_err(|_| EnvironmentCreateError::WorkerFailed)
}
fn event_id() -> String {
    // The ledger's existing event identity is twelve lowercase hexadecimal digits.
    format!("{:012x}", capsem_crypto::random::u64() & 0xffff_ffff_ffff)
}
fn view(record: EnvironmentRequestRecord) -> EnvironmentMutationResponse {
    let result = record.result.map(|result| EnvironmentRevisionReference {
        environment_id: result.environment_id,
        revision_id: result.revision_id,
    });
    EnvironmentMutationResponse {
        request_id: record.request_id,
        environment_id: record.environment_id,
        decision: match record.state {
            EnvironmentRequestState::Pending => RequestState::Pending,
            EnvironmentRequestState::Approved => RequestState::Approved,
            EnvironmentRequestState::Denied => RequestState::Denied,
            EnvironmentRequestState::Cancelled => RequestState::Cancelled,
            EnvironmentRequestState::Expired => RequestState::Expired,
            EnvironmentRequestState::Stale => RequestState::Stale,
        },
        application: if result.is_some() {
            RequestApplicationState::Applied
        } else {
            RequestApplicationState::Pending
        },
        result,
        application_error: None,
    }
}
#[cfg(test)]
mod tests;
