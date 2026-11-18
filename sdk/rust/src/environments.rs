//! Canonical blueprints through the authenticated gateway.
use crate::client::Client;
use crate::{models, operations as api, Error, Result};

pub struct Environments<'a>(pub(crate) &'a Client);
impl Environments<'_> {
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
    /// Submit a change against its observed base, with explicit reconciliation.
    pub async fn change(
        &self,
        id: &str,
        request: models::EnvironmentChangeRequest,
    ) -> Result<models::EnvironmentMutationResponse> {
        let input = api::ChangeEnvironmentParams {
            id: id.into(),
            body: request,
        };
        let result = api::change_environment(&self.0.transport, &input, self.0.options).await?;
        if result.request_id != input.body.request_id || result.environment_id != id {
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
}
#[cfg(test)]
mod tests;
