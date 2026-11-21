//! Canonical blueprints through the authenticated gateway.
use crate::client::Client;
use crate::{models, operations as api, Error, Result};
use models::environments::{EnvironmentRevisionListQuery, EnvironmentRevisionReference};

pub struct Environments<'a>(pub(crate) &'a Client);
impl Environments<'_> {
    /// List or search the committed environments visible to this credential.
    pub async fn list(
        &self,
        query: models::environments::EnvironmentListQuery,
    ) -> Result<models::EnvironmentListResponse> {
        query.validate().map_err(Error::InvalidInput)?;
        let input = api::ListEnvironmentsParams {
            search: query.search,
            after: query.after,
            limit: query.limit.map(u32::from),
        };
        api::list_environments(&self.0.transport, &input, self.0.options).await
    }

    /// Submit editable intent with a caller-chosen request ID. Reconcile that
    /// same ID explicitly after interruption; approval and application differ.
    pub async fn create(
        &self,
        request: models::EnvironmentCreateRequest,
    ) -> Result<models::EnvironmentMutationResponse> {
        let input = api::CreateEnvironmentParams { body: request };
        let result = api::create_environment(&self.0.transport, &input, self.0.options).await?;
        if result.request_id != input.body.request_id {
            return Err(Error::InvalidResponse("environment request identity changed"));
        }
        Ok(result)
    }
    /// Submit an edit against its observed base, with explicit reconciliation.
    pub async fn change(
        &self,
        id: &str,
        request: models::EnvironmentChangeRequest,
    ) -> Result<models::EnvironmentMutationResponse> {
        self.submit_change(id, models::EnvironmentChangeIntent::Edit(Box::new(request)))
            .await
    }

    /// Archive the exact observed head while preserving existing pinned sandboxes.
    pub async fn archive(
        &self,
        id: &str,
        request: models::environments::EnvironmentArchiveRequest,
    ) -> Result<models::EnvironmentMutationResponse> {
        self.submit_change(id, models::EnvironmentChangeIntent::Archive(request)).await
    }

    async fn submit_change(
        &self,
        id: &str,
        request: models::EnvironmentChangeIntent,
    ) -> Result<models::EnvironmentMutationResponse> {
        let input = api::ChangeEnvironmentParams {
            id: id.into(),
            body: request,
        };
        let result = api::change_environment(&self.0.transport, &input, self.0.options).await?;
        if result.request_id != input.body.request_id() || result.environment_id != id {
            return Err(Error::InvalidResponse("environment change identity changed"));
        }
        Ok(result)
    }

    /// Read the canonical current revision and its committed provenance.
    pub async fn info(&self, id: &str) -> Result<models::EnvironmentCurrentResponse> {
        let result = api::get_environment_info(
            &self.0.transport,
            &api::GetEnvironmentInfoParams { id: id.into() },
            self.0.options,
        )
        .await?;
        if result.document.identity.id != id {
            return Err(Error::InvalidResponse("environment identity changed"));
        }
        Ok(result)
    }

    pub async fn list_revisions(
        &self,
        id: &str,
        query: EnvironmentRevisionListQuery,
    ) -> Result<models::EnvironmentRevisionListResponse> {
        query.validated_limit().map_err(Error::InvalidInput)?;
        let input = api::ListEnvironmentRevisionsParams {
            id: id.into(),
            before: query.before,
            limit: query.limit.map(u32::from),
        };
        api::list_environment_revisions(&self.0.transport, &input, self.0.options).await
    }

    pub async fn revision(
        &self,
        reference: &EnvironmentRevisionReference,
    ) -> Result<models::EnvironmentRevisionDetailResponse> {
        reference.validate().map_err(Error::InvalidInput)?;
        let input = api::GetEnvironmentRevisionParams {
            id: reference.environment_id.clone(),
            revision_id: reference.revision_id.clone(),
        };
        let result = api::get_environment_revision(&self.0.transport, &input, self.0.options).await?;
        if result.summary.environment != *reference {
            return Err(Error::InvalidResponse("environment revision identity changed"));
        }
        Ok(result)
    }

    /// Read effective plugins for an exact current revision, or defaults for creation.
    pub async fn plugins(
        &self,
        reference: Option<&EnvironmentRevisionReference>,
    ) -> Result<models::EnvironmentPluginListResponse> {
        let result = if let Some(reference) = reference {
            reference.validate().map_err(Error::InvalidInput)?;
            let input = api::ListEnvironmentPluginsParams {
                id: reference.environment_id.clone(),
                revision_id: Some(reference.revision_id.clone()),
            };
            api::list_environment_plugins(&self.0.transport, &input, self.0.options).await?
        } else {
            api::list_new_environment_plugins(&self.0.transport, self.0.options).await?
        };
        if result.revision.as_ref() != reference {
            return Err(Error::InvalidResponse("environment plugin revision identity changed"));
        }
        Ok(result)
    }

    /// Read the rules compiled for an exact current environment revision.
    pub async fn rules(&self, reference: &EnvironmentRevisionReference) -> Result<models::EnvironmentRuleListResponse> {
        reference.validate().map_err(Error::InvalidInput)?;
        let input = api::ListEnvironmentRulesParams {
            id: reference.environment_id.clone(),
            revision_id: Some(reference.revision_id.clone()),
        };
        let result = api::list_environment_rules(&self.0.transport, &input, self.0.options).await?;
        if result.revision != *reference {
            return Err(Error::InvalidResponse("environment rule revision identity changed"));
        }
        Ok(result)
    }

    pub async fn validate_toml(
        &self,
        base: &EnvironmentRevisionReference,
        toml: String,
    ) -> Result<models::EnvironmentTomlValidationResponse> {
        base.validate().map_err(Error::InvalidInput)?;
        let input = api::ValidateEnvironmentTomlParams {
            id: base.environment_id.clone(),
            revision_id: base.revision_id.clone(),
            body: models::EnvironmentTomlValidationRequest { toml },
        };
        let result = api::validate_environment_toml(&self.0.transport, &input, self.0.options).await?;
        if result.base != *base {
            return Err(Error::InvalidResponse("environment revision identity changed"));
        }
        Ok(result)
    }
}
#[cfg(test)]
mod tests;
