//! Delegated header admission uses the coordinator's live capability registry.
use axum::http::{header::AUTHORIZATION, HeaderMap, StatusCode};
use capsem_proto::gateway_grant::auth::GatewayAuthSecret;

use crate::AppState;

pub(super) fn parse(candidate: &str) -> Option<([u8; 16], GatewayAuthSecret<32>)> {
    let (id, secret) = candidate.split_once('.')?;
    capsem_api::validate_managed_request_id(id).ok()?;
    let id = uuid::Uuid::parse_str(id).ok()?;
    let secret = capsem_api::ManagedOwnershipToken::new(secret.to_owned()).ok()?;
    Some((*id.as_bytes(), GatewayAuthSecret::from_bytes(secret.to_bytes())))
}

/// False means this is not a delegated header; it conveys no authority.
/// A candidate delegated header can never fall back to operator authentication.
pub(super) async fn authenticate(state: &AppState, headers: &HeaderMap) -> Result<bool, StatusCode> {
    if headers.get_all(AUTHORIZATION).iter().count() > 1 {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let Some(candidate) = headers
        .get(AUTHORIZATION)
        .and_then(|header| header.to_str().ok())
        .and_then(|header| header.strip_prefix("Bearer "))
    else {
        return Ok(false);
    };
    if !candidate.contains('.') {
        return Ok(false);
    }
    let (id, capability) = parse(candidate).ok_or(StatusCode::UNAUTHORIZED)?;
    match state.service_client.authenticate_agent(id, capability).await {
        Ok(true) => Ok(true),
        Ok(false) => Err(StatusCode::UNAUTHORIZED),
        Err(error) => {
            tracing::warn!(%error, "coordinator delegated authentication unavailable");
            Err(StatusCode::SERVICE_UNAVAILABLE)
        }
    }
}
