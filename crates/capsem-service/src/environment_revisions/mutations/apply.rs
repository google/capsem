//! Freeze the reviewed revision, selected owner generations and their bases.
use super::*;
use std::{collections::BTreeMap, future::Future};

type Error = EnvironmentMutationError;

pub enum OperatorEnvironmentApplication<T> {
    Observed(EnvironmentMutationResponse),
    Approved(ApprovedEnvironmentReview<Vec<T>>),
}

impl EnvironmentMutationCoordinator {
    pub async fn approve_application<T, F>(
        &self,
        environment_id: String,
        request: EnvironmentApplyRequest,
        validate: impl FnOnce(Box<EnvironmentSpecification>, Vec<EnvironmentApplyTarget>) -> F,
    ) -> Result<OperatorEnvironmentApplication<T>, Error>
    where
        F: Future<Output = Result<Vec<T>, Error>>,
    {
        request.validate().map_err(|_| Error::InvalidIntent)?;
        validate_id(&environment_id).map_err(|_| Error::InvalidIntent)?;
        let lock = self
            .revisions
            .lock_for(&format!("request:{}", request.request_id))
            .map_err(Error::Publication)?;
        let _request_guard = lock.lock().await;
        let base = EnvironmentRevisionReference {
            environment_id,
            revision_id: request.base_revision_id,
        };
        let revision = EnvironmentRevisionParent {
            environment_id: base.environment_id.clone(),
            revision_id: request.revision_id,
        };
        let mut targets = Vec::with_capacity(request.targets.len());
        let mut expected = BTreeMap::new();
        for input in &request.targets {
            targets.push(EnvironmentRequestTarget {
                session_id: input.session_id.clone(),
                generation: input.generation.clone(),
            });
            expected.insert(
                input.session_id.clone(),
                EnvironmentRevisionParent {
                    environment_id: input.base.environment_id.clone(),
                    revision_id: input.base.revision_id.clone(),
                },
            );
        }
        let payload = EnvironmentRequestPayload::Apply {
            revision: revision.clone(),
            targets,
            expected,
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
            let record = saved.record;
            if *value != payload
                || record.actor != "operator"
                || record.requester.is_some()
                || record.environment_id != base.environment_id
                || record.base_revision_id.as_deref() != Some(base.revision_id.as_str())
                || record.reason != request.reason
            {
                return Err(Error::Conflict);
            }
            return self
                .reconcile_view(record, "operator")
                .await
                .map(OperatorEnvironmentApplication::Observed);
        }

        self.load_base(&base).await?;
        let source = self
            .revisions
            .load_revision(&revision.environment_id, &revision.revision_id)
            .await
            .map_err(Error::Publication)?;
        let runtime = validate(source.specification, request.targets).await?;
        let env_lock = self
            .revisions
            .lock_for(&base.environment_id)
            .map_err(Error::Publication)?;
        let _environment_guard = env_lock.lock().await;
        self.revisions
            .reconcile_locked(&base.environment_id, now()?)
            .await
            .map_err(Error::Publication)?;
        if self
            .revisions
            .is_archived(&base.environment_id)
            .await
            .map_err(Error::Publication)?
        {
            return Err(Error::Archived);
        }
        let head = self
            .revisions
            .files
            .load_current(&self.revisions.ledger, &base.environment_id)
            .await
            .map_err(Error::Publication)?;
        if head.document.identity.revision_id != base.revision_id {
            return Err(Error::Conflict);
        }
        let at = now()?;
        self.write(WriteOp::PrepareEnvironmentRequest(PreparedEnvironmentRequest {
            request_id: request.request_id.clone(),
            environment_id: base.environment_id,
            base_revision_id: Some(base.revision_id),
            requester: None,
            actor: "operator".into(),
            timestamp_unix_ms: at,
            expires_at_unix_ms: expiry(at)?,
            reason: request.reason.clone(),
            payload_event_id: event_id(),
            payload: Box::new(payload),
        }))
        .await?;
        self.write(WriteOp::EnvironmentRequestDecision(EnvironmentRequestDecision {
            request_id: request.request_id.clone(),
            actor: "operator".into(),
            timestamp_unix_ms: now()?,
            reason: request.reason,
            decision: EnvironmentRequestDecisionKind::Approve,
        }))
        .await?;
        self.write(WriteOp::EnvironmentRequestResult(EnvironmentRequestResult {
            request_id: request.request_id.clone(),
            result: revision.clone(),
            actor: "operator".into(),
            timestamp_unix_ms: now()?,
        }))
        .await?;
        self.request_changed(&request.request_id);
        Ok(OperatorEnvironmentApplication::Approved(ApprovedEnvironmentReview {
            request_id: request.request_id,
            result: revision,
            runtime,
        }))
    }
}
