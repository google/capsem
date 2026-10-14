//! Explicit host consent and generation-bound grants through the authenticated gateway.

use reqwest::Method;
use serde::de::DeserializeOwned;

use crate::{client::Client, models, transport::Request, Error, Result, VM};

/// User-owned connections; connecting never grants a VM access implicitly.
#[derive(Debug, Clone, Copy)]
pub struct Connections<'a>(pub(crate) &'a Client);

/// Explicit approval for the canonical session resolved by this borrowed VM.
#[derive(Debug, Clone, Copy)]
pub struct CredentialGrants<'a>(pub(crate) &'a VM);

// These approved operations are awaiting live service/OpenAPI registration.
// Keep request execution in the existing transport: no discovery, browser,
// retry, token export, or separate HTTP connection pool.
async fn call<T: DeserializeOwned>(
    client: &Client,
    method: Method,
    path: &str,
    parameters: &[(&str, &str)],
    body: Option<Vec<u8>>,
) -> Result<T> {
    let bytes = client
        .transport
        .request(
            method,
            path,
            Request {
                parameters,
                body,
                options: client.options,
                ..Default::default()
            },
        )
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}

impl Connections<'_> {
    /// List redacted connections and configured capabilities. Consent alone
    /// does not establish that a native consumer supports credential handoff.
    pub async fn list(&self) -> Result<models::ConnectionListResponse> {
        call(self.0, Method::GET, "/connections", &[], None).await
    }

    /// Begin consent using a host-configured profile. The caller decides how
    /// to open the returned URL and when to poll; no browser is launched here.
    /// Retain the request ID for an explicit retry of the same request.
    pub async fn authorize_google(
        &self,
        request: models::AuthorizeGoogleRequest,
    ) -> Result<models::AuthorizeGoogleResponse> {
        call(
            self.0,
            Method::POST,
            "/connections/google/authorize",
            &[],
            Some(serde_json::to_vec(&request)?),
        )
        .await
    }

    pub async fn authorization(&self, id: &str) -> Result<models::AuthorizationTransaction> {
        let result: models::AuthorizationTransaction = call(
            self.0,
            Method::GET,
            "/connections/authorizations/{id}",
            &[("id", id)],
            None,
        )
        .await?;
        if result.id != id {
            return Err(Error::InvalidResponse("authorization identity changed"));
        }
        Ok(result)
    }

    pub async fn cancel_authorization(&self, id: &str) -> Result<models::AuthorizationTransaction> {
        let result: models::AuthorizationTransaction = call(
            self.0,
            Method::DELETE,
            "/connections/authorizations/{id}",
            &[("id", id)],
            None,
        )
        .await?;
        if result.id != id {
            return Err(Error::InvalidResponse("authorization identity changed"));
        }
        Ok(result)
    }

    pub async fn get(&self, id: &str) -> Result<models::ConnectionSummary> {
        let result: models::ConnectionSummary =
            call(self.0, Method::GET, "/connections/{id}", &[("id", id)], None).await?;
        if result.id != id {
            return Err(Error::InvalidResponse("connection identity changed"));
        }
        Ok(result)
    }

    /// Fence local grants and begin provider retirement. A `revoking` result
    /// requires explicit status polling; it does not confirm completed cleanup.
    pub async fn disconnect(&self, id: &str) -> Result<models::DisconnectResponse> {
        let result: models::DisconnectResponse =
            call(self.0, Method::DELETE, "/connections/{id}", &[("id", id)], None).await?;
        if result.connection.id != id {
            return Err(Error::InvalidResponse("connection identity changed"));
        }
        Ok(result)
    }
}

impl CredentialGrants<'_> {
    pub async fn list(&self) -> Result<models::SessionGrantListResponse> {
        let id = self.0.resolve().await?;
        let result: models::SessionGrantListResponse = call(
            &self.0.client,
            Method::GET,
            "/vms/{id}/credential-grants",
            &[("id", &id)],
            None,
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
        let result: models::SessionGrantSummary = call(
            &self.0.client,
            Method::POST,
            "/vms/{id}/credential-grants",
            &[("id", &id)],
            Some(serde_json::to_vec(&request)?),
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
        let result: models::RevokeSessionGrantResponse = call(
            &self.0.client,
            Method::DELETE,
            "/vms/{id}/credential-grants/{grant_id}",
            &[("id", &id), ("grant_id", grant_id)],
            None,
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
