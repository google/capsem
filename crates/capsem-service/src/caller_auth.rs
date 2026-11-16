//! Authenticated caller evidence. Only coordinator-owned bootstrap and kernel
//! peer admission may construct the proof required by service handlers.
use std::sync::{Arc, Mutex};

use axum::{
    extract::FromRequestParts,
    http::{request::Parts, HeaderMap, StatusCode},
};
use capsem_proto::gateway_grant::auth::{GatewayAuthDenial, GatewayAuthSecret};

#[derive(Default)]
pub(crate) struct Authority {
    operator: Mutex<Option<Operator>>,
}
struct Operator {
    epoch: uuid::Uuid,
    token: GatewayAuthSecret<64>,
}

pub(crate) struct OperatorEpoch {
    authority: Arc<Authority>,
    epoch: uuid::Uuid,
    registered: bool,
}

impl Authority {
    pub(crate) fn begin(self: &Arc<Self>) -> OperatorEpoch {
        OperatorEpoch {
            authority: Arc::clone(self),
            epoch: capsem_crypto::uuid_v4(),
            registered: false,
        }
    }

    pub(crate) fn authenticate_gateway(&self, headers: &HeaderMap) -> Result<AuthenticatedActor, StatusCode> {
        let candidate = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|header| header.to_str().ok())
            .and_then(|header| header.strip_prefix("Bearer "))
            .ok_or(StatusCode::UNAUTHORIZED)?;
        let operator = self.operator.lock().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let valid = operator.as_ref().is_some_and(|operator| {
            capsem_crypto::secrets::constant_time_eq(candidate.as_bytes(), operator.token.as_bytes())
        });
        drop(operator);
        if !valid {
            return Err(StatusCode::UNAUTHORIZED);
        }
        Ok(AuthenticatedActor(capsem_api::CallerActor::Operator))
    }
}

impl OperatorEpoch {
    pub(crate) fn register(&mut self, token: GatewayAuthSecret<64>) -> Result<(), GatewayAuthDenial> {
        if self.registered {
            return Err(GatewayAuthDenial::Forbidden);
        }
        let mut current = self
            .authority
            .operator
            .lock()
            .map_err(|_| GatewayAuthDenial::Unavailable)?;
        *current = Some(Operator {
            epoch: self.epoch,
            token,
        });
        drop(current);
        self.registered = true;
        Ok(())
    }
}

impl Drop for OperatorEpoch {
    fn drop(&mut self) {
        if let Ok(mut operator) = self.authority.operator.lock() {
            if operator.as_ref().is_some_and(|operator| operator.epoch == self.epoch) {
                *operator = None;
            }
        }
    }
}

/// The evidence DTO alone is not authority. This wrapper has no request-body
/// deserializer or caller-controlled constructor.
#[derive(Clone)]
pub(crate) struct AuthenticatedActor(capsem_api::CallerActor);

impl AuthenticatedActor {
    pub(crate) fn evidence(&self) -> &capsem_api::CallerActor {
        &self.0
    }
}
impl<S: Send + Sync> FromRequestParts<S> for AuthenticatedActor {
    type Rejection = StatusCode;
    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts.extensions.get::<Self>().cloned().ok_or(StatusCode::UNAUTHORIZED)
    }
}

#[cfg(test)]
mod tests;
