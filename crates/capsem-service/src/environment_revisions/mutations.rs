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
    Archived,
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
            Self::Archived => "environment is archived",
            Self::Resolution(error @ EnvironmentResolutionError::CatalogChanged) => {
                return std::fmt::Display::fmt(error, f)
            }
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
    notify_request: Option<RequestNotification>,
}
type RequestNotification = Box<dyn Fn(&str) + Send + Sync>;

impl EnvironmentMutationCoordinator {
    pub fn new(revisions: Arc<EnvironmentRevisions>) -> Self {
        Self {
            revisions,
            notify_request: None,
        }
    }

    pub fn with_request_notifications(mut self, notify: impl Fn(&str) + Send + Sync + 'static) -> Self {
        self.notify_request = Some(Box::new(notify));
        self
    }

    fn request_changed(&self, id: &str) {
        if let Some(notify) = &self.notify_request {
            notify(id);
        }
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

    pub fn submit_image_update<F>(
        self: &Arc<Self>,
        environment_id: String,
        request: EnvironmentImageUpdateRequest,
        resolve: impl FnOnce(EnvironmentImage) -> F + Send + 'static,
    ) -> Result<EnvironmentMutationJob, EnvironmentMutationError>
    where
        F: std::future::Future<Output = Result<Option<String>, EnvironmentResolutionError>> + Send + 'static,
    {
        request
            .validate()
            .map_err(|_| EnvironmentMutationError::InvalidIntent)?;
        validate_id(&environment_id).map_err(|_| EnvironmentMutationError::InvalidIntent)?;
        let owner = self.clone();
        Ok(EnvironmentMutationJob::spawn(async move {
            owner.update_image(environment_id, request, resolve).await
        }))
    }

    async fn update_image<F>(
        &self,
        environment_id: String,
        request: EnvironmentImageUpdateRequest,
        resolve: impl FnOnce(EnvironmentImage) -> F,
    ) -> Result<EnvironmentMutationResponse, EnvironmentMutationError>
    where
        F: std::future::Future<Output = Result<Option<String>, EnvironmentResolutionError>>,
    {
        use EnvironmentMutationError as Error;
        let lock = self
            .revisions
            .lock_for(&format!("request:{}", request.request_id))
            .map_err(Error::Publication)?;
        let _guard = lock.lock().await;
        let base = EnvironmentRevisionReference {
            environment_id,
            revision_id: request.base_revision_id,
        };
        if let Some(saved) = self
            .revisions
            .ledger
            .environment_request_history(&request.request_id)
            .await
            .map_err(|_| Error::LedgerFailure)?
        {
            let capsem_logger::db::EnvironmentHistoryPayload::Complete { value } = saved.payload else {
                return Err(Error::Conflict);
            };
            let EnvironmentRequestPayload::ImageUpdate { specification } = *value else {
                return Err(Error::Conflict);
            };
            let record = saved.record;
            if record.actor != "operator"
                || record.requester.is_some()
                || record.environment_id != base.environment_id
                || record.base_revision_id.as_deref() != Some(base.revision_id.as_str())
                || record.reason != request.reason
                || specification.image.as_ref().map(|image| image.resolved_digest.as_str())
                    != Some(request.expected_digest.as_str())
            {
                return Err(Error::Conflict);
            }
            return self.reconcile_view(record, "operator").await;
        }
        let mut current = self.load_base(&base).await?.document.specification;
        if self
            .revisions
            .is_archived(&base.environment_id)
            .await
            .map_err(Error::Publication)?
        {
            return Err(Error::Archived);
        }
        let image = current.image.as_mut().ok_or(Error::Conflict)?;
        let candidate = resolve(image.clone()).await.map_err(Error::Resolution)?;
        if candidate.as_deref() != Some(request.expected_digest.as_str())
            || request.expected_digest == image.resolved_digest
        {
            return Err(Error::Conflict);
        }
        image.resolved_digest = request.expected_digest;
        let result = self
            .publish_operator_revision(
                request.request_id,
                request.reason,
                Some(base),
                EnvironmentRequestPayload::ImageUpdate {
                    specification: Box::new(current),
                },
            )
            .await?;
        self.request_changed(&result.request_id);
        Ok(result)
    }

    pub fn submit_archive(
        self: &Arc<Self>,
        environment_id: String,
        request: EnvironmentArchiveRequest,
    ) -> Result<EnvironmentMutationJob, EnvironmentMutationError> {
        request
            .validate()
            .map_err(|_| EnvironmentMutationError::InvalidIntent)?;
        validate_id(&environment_id).map_err(|_| EnvironmentMutationError::InvalidIntent)?;
        let owner = self.clone();
        Ok(EnvironmentMutationJob::spawn(async move {
            owner.archive(environment_id, request).await
        }))
    }

    async fn archive(
        &self,
        environment_id: String,
        request: EnvironmentArchiveRequest,
    ) -> Result<EnvironmentMutationResponse, EnvironmentMutationError> {
        use EnvironmentMutationError as Error;
        let request_lock = self
            .revisions
            .lock_for(&format!("request:{}", request.request_id))
            .map_err(Error::Publication)?;
        let _request = request_lock.lock().await;
        let existing = self.find_request(request.request_id.clone()).await?;
        if let Some(record) = &existing {
            if record.kind != EnvironmentRequestKind::Archive
                || record.environment_id != environment_id
                || record.base_revision_id.as_ref() != Some(&request.base_revision_id)
                || record.reason != request.reason
            {
                return Err(Error::Conflict);
            }
            if record.state != EnvironmentRequestState::Pending {
                return Ok(view(record.clone()));
            }
        }
        let environment_lock = self.revisions.lock_for(&environment_id).map_err(Error::Publication)?;
        let _environment = environment_lock.lock().await;
        self.revisions
            .reconcile_locked(&environment_id, now()?)
            .await
            .map_err(Error::Publication)?;
        match current_revision(&self.revisions.ledger, &environment_id)
            .await
            .map_err(Error::Publication)?
        {
            None => return Err(Error::NotFound),
            Some(current) if current != request.base_revision_id => return Err(Error::Conflict),
            Some(_) => {}
        }
        if self
            .revisions
            .is_archived(&environment_id)
            .await
            .map_err(Error::Publication)?
        {
            return Err(Error::Archived);
        }
        let at = now()?;
        if existing.as_ref().is_some_and(|record| at >= record.expires_at_unix_ms) {
            self.write(WriteOp::EnvironmentRequestInvalidation(
                EnvironmentRequestInvalidation {
                    request_id: request.request_id.clone(),
                    timestamp_unix_ms: at,
                    actor: "operator".into(),
                    kind: EnvironmentRequestInvalidationKind::Expired,
                    reason: "archive request expired before its decision".into(),
                },
            ))
            .await?;
            self.request_changed(&request.request_id);
            return Ok(view(self.read_request(request.request_id).await?));
        }
        if existing.is_none() {
            self.write(WriteOp::PrepareEnvironmentRequest(PreparedEnvironmentRequest {
                request_id: request.request_id.clone(),
                environment_id,
                base_revision_id: Some(request.base_revision_id),
                requester: None,
                timestamp_unix_ms: at,
                expires_at_unix_ms: expiry(at)?,
                actor: "operator".into(),
                reason: request.reason.clone(),
                payload_event_id: event_id(),
                payload: Box::new(EnvironmentRequestPayload::Archive {}),
            }))
            .await?;
        }
        self.write(WriteOp::EnvironmentRequestDecision(EnvironmentRequestDecision {
            request_id: request.request_id.clone(),
            timestamp_unix_ms: now()?,
            actor: "operator".into(),
            decision: EnvironmentRequestDecisionKind::Approve,
            reason: request.reason,
        }))
        .await?;
        self.request_changed(&request.request_id);
        Ok(view(self.read_request(request.request_id).await?))
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
        let base = match &request.base {
            Some(expected) => Some(self.load_base(expected).await?),
            None => None,
        };
        let specification = resolve_environment(
            request.specification,
            base.as_ref().map(|base| &base.document.specification),
            EnvironmentResolutionMode::Edit,
            &sources,
        )
        .await
        .map_err(EnvironmentMutationError::Resolution)?;
        self.publish_operator_revision(
            request.request_id,
            request.reason,
            request.base,
            EnvironmentRequestPayload::Change {
                specification: Box::new(specification),
            },
        )
        .await
    }

    async fn load_base(
        &self,
        expected: &EnvironmentRevisionReference,
    ) -> Result<CurrentEnvironment, EnvironmentMutationError> {
        use EnvironmentMutationError as Error;
        match current_revision(&self.revisions.ledger, &expected.environment_id)
            .await
            .map_err(Error::Publication)?
        {
            None => return Err(Error::NotFound),
            Some(current) if current != expected.revision_id => return Err(Error::Conflict),
            Some(_) => {}
        }
        let current = self
            .revisions
            .load_current(&expected.environment_id)
            .await
            .map_err(Error::Publication)?;
        if current.document.identity.revision_id != expected.revision_id {
            return Err(Error::Conflict);
        }
        Ok(current)
    }

    async fn publish_operator_revision(
        &self,
        request_id: String,
        reason: String,
        base: Option<EnvironmentRevisionReference>,
        payload: EnvironmentRequestPayload,
    ) -> Result<EnvironmentMutationResponse, EnvironmentMutationError> {
        let environment_id = base
            .as_ref()
            .map(|base| base.environment_id.clone())
            .unwrap_or_else(|| capsem_crypto::uuid_v4().to_string());
        // Network resolution holds only the request lock. Serialize the final
        // base check, durable decision and publication under the existing
        // environment lock so two edits cannot both approve the same base.
        let environment_lock = base
            .as_ref()
            .map(|_| self.revisions.lock_for(&environment_id))
            .transpose()
            .map_err(EnvironmentMutationError::Publication)?;
        let (environment_guard, fork_parent) = if let Some(lock) = &environment_lock {
            let guard = lock.lock().await;
            self.revisions
                .reconcile_locked(&environment_id, now()?)
                .await
                .map_err(EnvironmentMutationError::Publication)?;
            if self
                .revisions
                .is_archived(&environment_id)
                .await
                .map_err(EnvironmentMutationError::Publication)?
            {
                return Err(EnvironmentMutationError::Archived);
            }
            let current = self
                .revisions
                .files
                .load_current(&self.revisions.ledger, &environment_id)
                .await
                .map_err(EnvironmentMutationError::Publication)?;
            if base
                .as_ref()
                .is_none_or(|base| current.document.identity.revision_id != base.revision_id)
            {
                return Err(EnvironmentMutationError::Conflict);
            }
            (Some(guard), current.revision.fork_parent)
        } else {
            (None, None)
        };
        let at = now()?;
        let kind = payload.kind();
        self.write(WriteOp::PrepareEnvironmentRequest(PreparedEnvironmentRequest {
            request_id: request_id.clone(),
            environment_id: environment_id.clone(),
            base_revision_id: base.as_ref().map(|base| base.revision_id.clone()),
            requester: None,
            timestamp_unix_ms: at,
            // Operator intent is approved immediately, never held for human waiting.
            expires_at_unix_ms: expiry(at)?,
            actor: "operator".into(),
            reason: reason.clone(),
            payload_event_id: event_id(),
            payload: Box::new(payload),
        }))
        .await?;
        // Approval and publication consume the durable frozen request, not a
        // second resolution or a cloned mutable request payload.
        let saved = self
            .revisions
            .ledger
            .environment_request_snapshot(&request_id)
            .await
            .map_err(|_| EnvironmentMutationError::LedgerFailure)?
            .ok_or(EnvironmentMutationError::LedgerFailure)?;
        if saved.payload.kind() != kind {
            return Err(EnvironmentMutationError::Conflict);
        }
        let specification = match *saved.payload {
            EnvironmentRequestPayload::Change { specification }
            | EnvironmentRequestPayload::ImageUpdate { specification } => specification,
            _ => return Err(EnvironmentMutationError::Conflict),
        };
        self.write(WriteOp::EnvironmentRequestDecision(EnvironmentRequestDecision {
            request_id: request_id.clone(),
            timestamp_unix_ms: now()?,
            actor: "operator".into(),
            decision: EnvironmentRequestDecisionKind::Approve,
            reason: reason.clone(),
        }))
        .await?;
        let at = now()?;
        let revision = PreparedEnvironmentRevision {
            environment_id,
            revision_id: capsem_crypto::uuid_v4().to_string(),
            previous_revision_id: base.as_ref().map(|base| base.revision_id.clone()),
            fork_parent,
            timestamp_unix_ms: at,
            actor: "operator".into(),
            reason,
            request_id: Some(request_id.clone()),
            payload_event_id: event_id(),
            specification,
        };
        if let Some(guard) = &environment_guard {
            self.revisions.commit_locked(revision, at, CommitMode::New, guard).await
        } else {
            self.revisions.commit(revision, at).wait().await
        }
        .map_err(EnvironmentMutationError::Publication)?;
        Ok(view(self.read_request(request_id).await?))
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
        let record = saved.record;
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
            || !specification.matches_draft(&request.specification)
        {
            return Err(EnvironmentMutationError::Conflict);
        }
        self.reconcile_view(record, if origin.is_some() { "system" } else { "operator" })
            .await
            .map(Some)
    }

    async fn reconcile_view(
        &self,
        mut record: EnvironmentRequestRecord,
        expiry_actor: &str,
    ) -> Result<EnvironmentMutationResponse, EnvironmentMutationError> {
        let at = now()?;
        if record.state == EnvironmentRequestState::Pending && at >= record.expires_at_unix_ms {
            self.write(WriteOp::EnvironmentRequestInvalidation(
                EnvironmentRequestInvalidation {
                    request_id: record.request_id.clone(),
                    timestamp_unix_ms: at,
                    actor: expiry_actor.into(),
                    kind: EnvironmentRequestInvalidationKind::Expired,
                    reason: "review deadline expired before an operator decision".into(),
                },
            ))
            .await?;
            self.request_changed(&record.request_id);
            record = self.read_request(record.request_id).await?;
        }
        // Pending, interrupted or stale requests remain visible. Reconciliation
        // never casts another vote, creates a revision, or retries application.
        Ok(view(record))
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
