//! Publish the reviewed fork contents as a new revision of its parent.
use super::*;
use capsem_logger::db::EnvironmentHistoryPayload;
use std::future::Future;

type Error = EnvironmentMutationError;

impl EnvironmentMutationCoordinator {
    pub fn submit_promotion<F>(
        self: &Arc<Self>,
        environment_id: String,
        request: EnvironmentPromotionRequest,
        validate: impl FnOnce(Box<EnvironmentSpecification>) -> F + Send + 'static,
    ) -> Result<EnvironmentMutationJob, Error>
    where
        F: Future<Output = Result<(), Error>> + Send + 'static,
    {
        request.validate().map_err(|_| Error::InvalidIntent)?;
        validate_id(&environment_id).map_err(|_| Error::InvalidIntent)?;
        let owner = self.clone();
        Ok(EnvironmentMutationJob::spawn(async move {
            owner.promote(environment_id, request, validate).await
        }))
    }

    async fn promote<F>(
        &self,
        environment_id: String,
        request: EnvironmentPromotionRequest,
        validate: impl FnOnce(Box<EnvironmentSpecification>) -> F,
    ) -> Result<EnvironmentMutationResponse, Error>
    where
        F: Future<Output = Result<(), Error>>,
    {
        let lock = self
            .revisions
            .lock_for(&format!("request:{}", request.request_id))
            .map_err(Error::Publication)?;
        let _guard = lock.lock().await;
        let base = EnvironmentRevisionReference {
            environment_id,
            revision_id: request.base_revision_id,
        };
        let source = EnvironmentRevisionParent {
            environment_id: request.source.environment_id,
            revision_id: request.source.revision_id,
        };
        if let Some(saved) = self
            .revisions
            .ledger
            .environment_request_history(&request.request_id)
            .await
            .map_err(|_| Error::LedgerFailure)?
        {
            let EnvironmentHistoryPayload::Complete { value } = saved.payload else {
                return Err(Error::Conflict);
            };
            let record = saved.record;
            if !matches!(*value, EnvironmentRequestPayload::Promotion { source: previous } if previous == source)
                || record.actor != "operator"
                || record.requester.is_some()
                || record.environment_id != base.environment_id
                || record.base_revision_id.as_deref() != Some(base.revision_id.as_str())
                || record.reason != request.reason
            {
                return Err(Error::Conflict);
            }
            return self.reconcile_view(record, "operator").await;
        }

        self.load_base(&base).await?;
        if self
            .revisions
            .is_archived(&base.environment_id)
            .await
            .map_err(Error::Publication)?
        {
            return Err(Error::Archived);
        }
        let fork = self
            .revisions
            .load_revision(&source.environment_id, &source.revision_id)
            .await
            .map_err(Error::Publication)?;
        if !fork
            .record
            .fork_parent
            .is_some_and(|parent| parent.environment_id == base.environment_id)
        {
            return Err(Error::Conflict);
        }
        validate(fork.specification).await?;
        let result = self
            .publish_operator_revision(
                request.request_id,
                request.reason,
                Some(base),
                EnvironmentRequestPayload::Promotion { source },
            )
            .await?;
        self.request_changed(&result.request_id);
        Ok(result)
    }
}
