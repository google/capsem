//! Agent proposals record frozen intent for review under an exact VM generation.
use super::*;

pub struct EnvironmentProposalOrigin {
    actor: String,
    target: EnvironmentRequestTarget,
}

impl EnvironmentProposalOrigin {
    /// The caller derives these identities from its authenticated grant and owner.
    pub fn delegated(access_id: uuid::Uuid, session_id: String, generation: uuid::Uuid) -> Self {
        Self {
            actor: format!("agent:{access_id}"),
            target: EnvironmentRequestTarget {
                session_id,
                generation: generation.to_string(),
            },
        }
    }

    pub(super) fn actor(&self) -> &str {
        &self.actor
    }

    pub(super) fn target(&self) -> &EnvironmentRequestTarget {
        &self.target
    }
}

impl EnvironmentMutationCoordinator {
    /// The service supplies live authorization and the owning generation.
    pub fn submit_proposal(
        self: &Arc<Self>,
        environment_id: String,
        request: EnvironmentChangeRequest,
        origin: EnvironmentProposalOrigin,
        sources: impl EnvironmentPinResolver + Send + 'static,
        authorize: impl Fn(&EnvironmentRevisionReference) -> Result<(), EnvironmentMutationError> + Send + 'static,
    ) -> Result<EnvironmentMutationJob, EnvironmentMutationError> {
        let request = MutationIntent::change(environment_id, request)?;
        let owner = self.clone();
        Ok(EnvironmentMutationJob::spawn(async move {
            owner.prepare_proposal(request, origin, sources, authorize).await
        }))
    }

    async fn prepare_proposal(
        &self,
        request: MutationIntent,
        origin: EnvironmentProposalOrigin,
        sources: impl EnvironmentPinResolver,
        authorize: impl Fn(&EnvironmentRevisionReference) -> Result<(), EnvironmentMutationError>,
    ) -> Result<EnvironmentMutationResponse, EnvironmentMutationError> {
        let lock = self
            .revisions
            .lock_for(&format!("request:{}", request.request_id))
            .map_err(EnvironmentMutationError::Publication)?;
        let _guard = lock.lock().await;
        let base = request.base.as_ref().ok_or(EnvironmentMutationError::InvalidIntent)?;
        authorize(base)?;
        let replay = self.reconcile(&request, Some(&origin)).await;
        authorize(base)?;
        if let Some(saved) = replay? {
            return Ok(saved);
        }
        let original = self
            .revisions
            .load_revision(&base.environment_id, &base.revision_id)
            .await
            .map_err(EnvironmentMutationError::Publication)?;
        authorize(base)?;
        let specification = resolve_environment(
            request.specification,
            Some(&original.specification),
            EnvironmentResolutionMode::Edit,
            &sources,
        )
        .await;
        authorize(base)?;
        let specification = specification.map_err(EnvironmentMutationError::Resolution)?;
        let at = now()?;
        self.write(WriteOp::PrepareEnvironmentRequest(PreparedEnvironmentRequest {
            request_id: request.request_id.clone(),
            environment_id: base.environment_id.clone(),
            base_revision_id: Some(base.revision_id.clone()),
            requester: Some(origin.target),
            timestamp_unix_ms: at,
            expires_at_unix_ms: expiry(at)?,
            actor: origin.actor,
            reason: request.reason,
            payload_event_id: event_id(),
            payload: Box::new(EnvironmentRequestPayload::Change {
                specification: Box::new(specification),
            }),
        }))
        .await?;
        Ok(view(self.read_request(request.request_id).await?))
    }
}
