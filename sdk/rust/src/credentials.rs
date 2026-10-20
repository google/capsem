use crate::{client::Client, models, operations as api, Error, Result};

/// Register host credentials and supply their opaque references to workloads.
#[derive(Debug, Clone, Copy)]
pub struct Credentials<'a>(pub(crate) &'a Client);

impl Credentials<'_> {
    /// Memory values last for the service lifetime. Supplied OAuth tokens are
    /// not automatically refreshed, and injection never initiates consent.
    pub async fn inject(
        &self,
        provider: models::CredentialInjectProvider,
        value: &str,
        storage: models::CredentialStorage,
    ) -> Result<models::CredentialInjectResponse> {
        if value.is_empty() {
            return Err(Error::InvalidInput("credential value must be nonempty"));
        }
        api::inject_credential(
            &self.0.transport,
            &api::InjectCredentialParams {
                body: models::CredentialInjectRequest {
                    provider,
                    value: value.to_owned(),
                    storage,
                },
            },
            self.0.options,
        )
        .await
    }
}

#[cfg(test)]
mod tests;
