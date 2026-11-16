//! Recognition for audit is deliberately separate from request authority.
use super::*;
use capsem_core::managed_sessions::Capability;

pub(super) fn bearer(headers: &HeaderMap) -> Result<&str, StatusCode> {
    if headers.get_all(axum::http::header::AUTHORIZATION).iter().count() != 1 {
        return Err(StatusCode::UNAUTHORIZED);
    }
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|header| header.to_str().ok())
        .and_then(|header| header.strip_prefix("Bearer "))
        .ok_or(StatusCode::UNAUTHORIZED)
}

pub(super) fn delegated(candidate: &str) -> Result<(uuid::Uuid, Capability), StatusCode> {
    let (id, token) = candidate.split_once('.').ok_or(StatusCode::UNAUTHORIZED)?;
    capsem_api::validate_managed_request_id(id).map_err(|_| StatusCode::UNAUTHORIZED)?;
    let id = uuid::Uuid::parse_str(id).map_err(|_| StatusCode::UNAUTHORIZED)?;
    let token = capsem_api::ManagedOwnershipToken::new(token.to_owned()).map_err(|_| StatusCode::UNAUTHORIZED)?;
    Ok((id, Capability::from_bytes(token.to_bytes())))
}

impl Authority {
    /// Closed capabilities may identify a refusal, never authorize a request.
    /// Unknown secrets return no verified identity, even with a known UUID.
    pub(crate) fn refusal_actor(&self, headers: &HeaderMap) -> Result<Option<capsem_api::CallerActor>, StatusCode> {
        let Ok(candidate) = bearer(headers) else {
            return Ok(None);
        };
        if candidate.contains('.') {
            let Ok((id, capability)) = delegated(candidate) else {
                return Ok(None);
            };
            return self
                .agents
                .recognizes(id, &capability)
                .map(|recognized| {
                    recognized.then(|| capsem_api::CallerActor::DelegatedAgent {
                        access_id: id.to_string(),
                    })
                })
                .map_err(|_| StatusCode::SERVICE_UNAVAILABLE);
        }
        match self.authenticate_gateway(headers) {
            Ok(actor) => Ok(Some(actor.evidence().clone())),
            Err(StatusCode::UNAUTHORIZED) => Ok(None),
            Err(status) => Err(status),
        }
    }
}
