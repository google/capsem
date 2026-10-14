//! Explicit host consent and generation-bound grants through the authenticated gateway.

use crate::{client::Client, models, operations as api, Error, Result, VM};

/// User-owned connections; connecting never grants a VM access implicitly.
#[derive(Debug, Clone, Copy)]
pub struct Connections<'a>(pub(crate) &'a Client);

/// Explicit approval for the canonical session resolved by this borrowed VM.
#[derive(Debug, Clone, Copy)]
pub struct CredentialGrants<'a>(pub(crate) &'a VM);

impl Connections<'_> {
    /// List redacted connections and configured capabilities. Consent alone
    /// does not establish that a native consumer supports credential handoff.
    pub async fn list(&self) -> Result<models::ConnectionListResponse> {
        api::list_connections(&self.0.transport, self.0.options).await
    }

    /// Begin consent using a host-configured profile. The caller decides how
    /// to open the returned URL and when to poll; no browser is launched here.
    /// Retain the request ID for an explicit retry of the same request.
    pub async fn authorize_google(
        &self,
        request: models::AuthorizeGoogleRequest,
    ) -> Result<models::AuthorizeGoogleResponse> {
        api::authorize_google_connection(
            &self.0.transport,
            &api::AuthorizeGoogleConnectionParams { body: request },
            self.0.options,
        )
        .await
    }

    pub async fn authorization(&self, id: &str) -> Result<models::AuthorizationTransaction> {
        let result = api::get_connection_authorization(
            &self.0.transport,
            &api::GetConnectionAuthorizationParams {
                transaction_id: id.into(),
            },
            self.0.options,
        )
        .await?;
        if result.id != id {
            return Err(Error::InvalidResponse("authorization identity changed"));
        }
        Ok(result)
    }

    pub async fn cancel_authorization(&self, id: &str) -> Result<models::AuthorizationTransaction> {
        let result = api::cancel_connection_authorization(
            &self.0.transport,
            &api::CancelConnectionAuthorizationParams {
                transaction_id: id.into(),
            },
            self.0.options,
        )
        .await?;
        if result.id != id {
            return Err(Error::InvalidResponse("authorization identity changed"));
        }
        Ok(result)
    }

    pub async fn get(&self, id: &str) -> Result<models::ConnectionSummary> {
        let result = api::get_connection(
            &self.0.transport,
            &api::GetConnectionParams {
                connection_id: id.into(),
            },
            self.0.options,
        )
        .await?;
        if result.id != id {
            return Err(Error::InvalidResponse("connection identity changed"));
        }
        Ok(result)
    }

    /// Fence local grants and begin provider retirement. A `revoking` result
    /// requires explicit status polling; it does not confirm completed cleanup.
    pub async fn disconnect(&self, id: &str) -> Result<models::DisconnectResponse> {
        let result = api::disconnect_connection(
            &self.0.transport,
            &api::DisconnectConnectionParams {
                connection_id: id.into(),
            },
            self.0.options,
        )
        .await?;
        if result.connection.id != id {
            return Err(Error::InvalidResponse("connection identity changed"));
        }
        Ok(result)
    }
}

impl CredentialGrants<'_> {
    pub async fn list(&self) -> Result<models::SessionGrantListResponse> {
        let id = self.0.resolve().await?;
        let result = api::list_session_credential_grants(
            &self.0.client.transport,
            &api::ListSessionCredentialGrantsParams { id: id.clone() },
            self.0.client.options,
        )
        .await?;
        if result.session_id != id
            || result
                .grants
                .iter()
                .any(|grant| grant.session_id != id || grant.owner_generation != result.owner_generation)
        {
            return Err(Error::InvalidResponse("grant session or ownership changed"));
        }
        Ok(result)
    }

    /// Explicitly approve configured capabilities for this ownership generation.
    /// This never reconnects, acquires consent, or retries a mutation implicitly.
    pub async fn create(&self, request: models::CreateSessionGrantRequest) -> Result<models::SessionGrantSummary> {
        let id = self.0.resolve().await?;
        let result = api::create_session_credential_grant(
            &self.0.client.transport,
            &api::CreateSessionCredentialGrantParams {
                id: id.clone(),
                body: request.clone(),
            },
            self.0.client.options,
        )
        .await?;
        let requested: std::collections::BTreeSet<_> = request.capability_ids.iter().collect();
        let returned: std::collections::BTreeSet<_> = result.capability_ids.iter().collect();
        if result.session_id != id
            || result.owner_generation != request.expected_owner_generation
            || result.connection_id != request.connection_id
            || returned != requested
            || returned.len() != result.capability_ids.len()
        {
            return Err(Error::InvalidResponse(
                "grant session, ownership, connection or capabilities changed",
            ));
        }
        Ok(result)
    }

    pub async fn revoke(&self, grant_id: &str) -> Result<models::RevokeSessionGrantResponse> {
        let id = self.0.resolve().await?;
        let result = api::revoke_session_credential_grant(
            &self.0.client.transport,
            &api::RevokeSessionCredentialGrantParams {
                id: id.clone(),
                grant_id: grant_id.into(),
            },
            self.0.client.options,
        )
        .await?;
        if result.grant.session_id != id || result.grant.id != grant_id {
            return Err(Error::InvalidResponse("revoked grant identity changed"));
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests;
