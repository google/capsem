//! Trusted environment mutation coordination. Authenticated routes admit the caller before
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

mod proposals;
pub use proposals::EnvironmentProposalOrigin;
mod decisions;
pub use decisions::{ApprovedEnvironmentReview, ResolvedEnvironmentReview, ReviewedEnvironmentChange};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvironmentMutationError {
    InvalidIntent,
    Refused,
    NotFound,
    Conflict,
    Resolution(EnvironmentResolutionError),
    LedgerFailure,
    Publication(EnvironmentFileError),
    WorkerFailed,
}
impl std::fmt::Display for EnvironmentMutationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotFound => "environment does not exist",
            Self::InvalidIntent => "invalid environment mutation intent",
            Self::Refused => "delegated session scope ended or does not cover this operation and owner",
            Self::Conflict => "request identity already has different or non-replayable intent",
            Self::Resolution(_) => "environment source resolution failed",
            Self::LedgerFailure => "environment request durability is unconfirmed",
            Self::Publication(_) => "environment publication is unconfirmed; reconcile the request",
            Self::WorkerFailed => "environment mutation worker failed",
        })
    }
}
impl std::error::Error for EnvironmentMutationError {}

pub struct EnvironmentMutationJob {
    receiver: oneshot::Receiver<Result<EnvironmentMutationResponse, EnvironmentMutationError>>,
}
impl EnvironmentMutationJob {
    fn spawn(
        work: impl std::future::Future<Output = Result<EnvironmentMutationResponse, EnvironmentMutationError>>
            + Send
            + 'static,
    ) -> Self {
        let (sender, receiver) = oneshot::channel();
        tokio::spawn(async move {
            let _ = sender.send(work.await);
        });
        Self { receiver }
    }

    pub async fn wait(self) -> Result<EnvironmentMutationResponse, EnvironmentMutationError> {
        self.receiver
            .await
            .map_err(|_| EnvironmentMutationError::WorkerFailed)?
    }
}

struct MutationIntent {
    request_id: String,
    specification: EnvironmentDraft,
    reason: String,
    base: Option<EnvironmentRevisionReference>,
}

impl MutationIntent {
    fn change(environment_id: String, request: EnvironmentChangeRequest) -> Result<Self, EnvironmentMutationError> {
        request
            .validate()
            .map_err(|_| EnvironmentMutationError::InvalidIntent)?;
        validate_id(&environment_id).map_err(|_| EnvironmentMutationError::InvalidIntent)?;
        if request.requesting_sandbox.is_some() {
            return Err(EnvironmentMutationError::InvalidIntent);
        }
        Ok(Self {
            request_id: request.request_id,
            specification: request.specification,
            reason: request.reason,
            base: Some(EnvironmentRevisionReference {
                environment_id,
                revision_id: request.base_revision_id,
            }),
        })
    }
}

pub struct EnvironmentMutationCoordinator {
    revisions: Arc<EnvironmentRevisions>,
}
impl EnvironmentMutationCoordinator {
    pub fn new(revisions: Arc<EnvironmentRevisions>) -> Self {
        Self { revisions }
    }

    pub async fn request_status(&self, id: String) -> Result<EnvironmentMutationResponse, EnvironmentMutationError> {
        self.read_request(id).await.map(view)
    }

    /// Dropping the HTTP response or this job cannot cancel accepted work.
    /// The source capability is selected by the host, never deserialized.
    pub fn submit<R>(
        self: &Arc<Self>,
        request: EnvironmentCreateRequest,
        sources: R,
    ) -> Result<EnvironmentMutationJob, EnvironmentMutationError>
    where
        R: EnvironmentPinResolver + Send + 'static,
    {
        request
            .validate()
            .map_err(|_| EnvironmentMutationError::InvalidIntent)?;
        if request.requesting_sandbox.is_some() {
            return Err(EnvironmentMutationError::InvalidIntent);
        }
        self.enqueue(
            MutationIntent {
                request_id: request.request_id,
                specification: request.specification,
                reason: request.reason,
                base: None,
            },
            sources,
        )
    }

    pub fn submit_change<R>(
        self: &Arc<Self>,
        environment_id: String,
        request: EnvironmentChangeRequest,
        sources: R,
    ) -> Result<EnvironmentMutationJob, EnvironmentMutationError>
    where
        R: EnvironmentPinResolver + Send + 'static,
    {
        self.enqueue(MutationIntent::change(environment_id, request)?, sources)
    }

    fn enqueue<R>(
        self: &Arc<Self>,
        request: MutationIntent,
        sources: R,
    ) -> Result<EnvironmentMutationJob, EnvironmentMutationError>
    where
        R: EnvironmentPinResolver + Send + 'static,
    {
        let owner = self.clone();
        Ok(EnvironmentMutationJob::spawn(async move {
            owner.mutate(request, sources).await
        }))
    }

    async fn mutate(
        &self,
        request: MutationIntent,
        sources: impl EnvironmentPinResolver,
    ) -> Result<EnvironmentMutationResponse, EnvironmentMutationError> {
        // A request namespace shares the existing owner's keyed lock lifetime,
        // without holding a policy/environment lock during network resolution.
        let lock = self
            .revisions
            .lock_for(&format!("request:{}", request.request_id))
            .map_err(EnvironmentMutationError::Publication)?;
        let _guard = lock.lock().await;
        if let Some(saved) = self.reconcile(&request, None).await? {
            return Ok(saved);
        }
        let base = if let Some(expected) = &request.base {
            match current_revision(&self.revisions.ledger, &expected.environment_id)
                .await
                .map_err(EnvironmentMutationError::Publication)?
            {
                None => return Err(EnvironmentMutationError::NotFound),
                Some(current) if current != expected.revision_id => return Err(EnvironmentMutationError::Conflict),
                Some(_) => {}
            }
            let current = self
                .revisions
                .load_current(&expected.environment_id)
                .await
                .map_err(EnvironmentMutationError::Publication)?;
            if current.document.identity.revision_id != expected.revision_id {
                return Err(EnvironmentMutationError::Conflict);
            }
            Some(current)
        } else {
            None
        };
        let specification = resolve_environment(
            request.specification,
            base.as_ref().map(|base| &base.document.specification),
            EnvironmentResolutionMode::Edit,
            &sources,
        )
        .await
        .map_err(EnvironmentMutationError::Resolution)?;
        let environment_id = request
            .base
            .as_ref()
            .map(|base| base.environment_id.clone())
            .unwrap_or_else(|| capsem_crypto::uuid_v4().to_string());
        // Network resolution holds only the request lock. Serialize the final
        // base check, durable decision and publication under the existing
        // environment lock so two edits cannot both approve the same base.
        let environment_lock = request
            .base
            .as_ref()
            .map(|_| self.revisions.lock_for(&environment_id))
            .transpose()
            .map_err(EnvironmentMutationError::Publication)?;
        let environment_guard = if let Some(lock) = &environment_lock {
            let guard = lock.lock().await;
            self.revisions
                .reconcile_locked(&environment_id, now()?)
                .await
                .map_err(EnvironmentMutationError::Publication)?;
            let current = self
                .revisions
                .files
                .load_current(&self.revisions.ledger, &environment_id)
                .await
                .map_err(EnvironmentMutationError::Publication)?;
            if request
                .base
                .as_ref()
                .is_none_or(|base| current.document.identity.revision_id != base.revision_id)
            {
                return Err(EnvironmentMutationError::Conflict);
            }
            Some(guard)
        } else {
            None
        };
        let at = now()?;
        self.write(WriteOp::PrepareEnvironmentRequest(PreparedEnvironmentRequest {
            request_id: request.request_id.clone(),
            environment_id: environment_id.clone(),
            base_revision_id: request.base.as_ref().map(|base| base.revision_id.clone()),
            requester: None,
            timestamp_unix_ms: at,
            // Operator intent is approved immediately, never held for human waiting.
            expires_at_unix_ms: expiry(at)?,
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
            .map_err(|_| EnvironmentMutationError::LedgerFailure)?
            .ok_or(EnvironmentMutationError::LedgerFailure)?;
        let EnvironmentRequestPayload::Change { specification } = *saved.payload else {
            return Err(EnvironmentMutationError::Conflict);
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
        let revision = PreparedEnvironmentRevision {
            environment_id,
            revision_id: capsem_crypto::uuid_v4().to_string(),
            previous_revision_id: request.base.as_ref().map(|base| base.revision_id.clone()),
            fork_parent: None,
            timestamp_unix_ms: at,
            actor: "operator".into(),
            reason: request.reason,
            request_id: Some(request.request_id.clone()),
            payload_event_id: event_id(),
            specification,
        };
        if let Some(guard) = &environment_guard {
            self.revisions.commit_locked(revision, at, CommitMode::New, guard).await
        } else {
            self.revisions.commit(revision, at).wait().await
        }
        .map_err(EnvironmentMutationError::Publication)?;
        Ok(view(self.read_request(request.request_id).await?))
    }

    async fn reconcile(
        &self,
        request: &MutationIntent,
        origin: Option<&EnvironmentProposalOrigin>,
    ) -> Result<Option<EnvironmentMutationResponse>, EnvironmentMutationError> {
        let Some(saved) = self
            .revisions
            .ledger
            .environment_request_history(&request.request_id)
            .await
            .map_err(|_| EnvironmentMutationError::LedgerFailure)?
        else {
            return Ok(None);
        };
        let capsem_logger::db::EnvironmentHistoryPayload::Complete { value } = saved.payload else {
            return Err(EnvironmentMutationError::Conflict);
        };
        let mut record = saved.record;
        let EnvironmentRequestPayload::Change { specification } = *value else {
            return Err(EnvironmentMutationError::Conflict);
        };
        if record.actor != origin.map_or("operator", EnvironmentProposalOrigin::actor)
            || record.requester.as_ref() != origin.map(EnvironmentProposalOrigin::target)
            || record.base_revision_id.as_deref() != request.base.as_ref().map(|base| base.revision_id.as_str())
            || request
                .base
                .as_ref()
                .is_some_and(|base| base.environment_id != record.environment_id)
            || record.reason != request.reason
            || specification.into_draft() != request.specification
        {
            return Err(EnvironmentMutationError::Conflict);
        }
        let at = now()?;
        if record.state == EnvironmentRequestState::Pending && at >= record.expires_at_unix_ms {
            self.write(WriteOp::EnvironmentRequestInvalidation(
                EnvironmentRequestInvalidation {
                    request_id: request.request_id.clone(),
                    timestamp_unix_ms: at,
                    actor: if origin.is_some() { "system" } else { "operator" }.into(),
                    kind: EnvironmentRequestInvalidationKind::Expired,
                    reason: "review deadline expired before an operator decision".into(),
                },
            ))
            .await?;
            record = self.read_request(request.request_id.clone()).await?;
        }
        // Pending, interrupted or stale requests remain visible. Reconciliation
        // never casts another vote, creates a revision, or retries application.
        Ok(Some(view(record)))
    }

    async fn read_request(&self, request_id: String) -> Result<EnvironmentRequestRecord, EnvironmentMutationError> {
        self.find_request(request_id)
            .await?
            .ok_or(EnvironmentMutationError::LedgerFailure)
    }

    async fn find_request(
        &self,
        request_id: String,
    ) -> Result<Option<EnvironmentRequestRecord>, EnvironmentMutationError> {
        self.revisions
            .ledger
            .environment_requests(EnvironmentRequestQuery::Get { request_id })
            .await
            .map(|mut rows| rows.pop())
            .map_err(|_| EnvironmentMutationError::LedgerFailure)
    }

    async fn write(&self, operation: WriteOp) -> Result<(), EnvironmentMutationError> {
        self.revisions
            .ledger
            .write(operation)
            .await
            .map_err(|_| EnvironmentMutationError::LedgerFailure)?;
        self.revisions
            .ledger
            .flush()
            .await
            .map_err(|_| EnvironmentMutationError::LedgerFailure)
    }
}
fn expiry(at: i64) -> Result<i64, EnvironmentMutationError> {
    i64::try_from(capsem_proto::APPROVAL_TIMEOUT_SECONDS)
        .ok()
        .and_then(|seconds| seconds.checked_mul(1000))
        .and_then(|duration| at.checked_add(duration))
        .ok_or(EnvironmentMutationError::WorkerFailed)
}

pub(super) fn now() -> Result<i64, EnvironmentMutationError> {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| EnvironmentMutationError::WorkerFailed)?
            .as_millis(),
    )
    .map_err(|_| EnvironmentMutationError::WorkerFailed)
}
fn event_id() -> String {
    // The ledger's existing event identity is twelve lowercase hexadecimal digits.
    format!("{:012x}", capsem_crypto::random::u64() & 0xffff_ffff_ffff)
}
pub(super) fn view(record: EnvironmentRequestRecord) -> EnvironmentMutationResponse {
    use capsem_logger::events::environment::application::EnvironmentApplicationState as Application;
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
        application: match record
            .application
            .as_ref()
            .map(|status| (status.state, status.pending_restart))
        {
            Some((Application::Applying, _)) => RequestApplicationState::Applying,
            Some((Application::Applied, false)) => RequestApplicationState::Applied,
            Some((Application::Failed, _)) => RequestApplicationState::Failed,
            None if result.is_some() && record.requester.is_none() => RequestApplicationState::Applied,
            _ => RequestApplicationState::Pending,
        },
        result,
        application_error: record.application.and_then(|status| status.failure_reason),
        observed_revision: record.requester.is_some().then_some(record.payload_event_id),
    }
}
#[cfg(test)]
mod tests;
