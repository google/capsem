//! Authenticated caller evidence. Only coordinator-owned bootstrap and kernel
//! peer admission may construct the proof required by service handlers.
use std::sync::{Arc, Mutex};

use axum::{
    extract::FromRequestParts,
    http::{request::Parts, HeaderMap, StatusCode},
};
use capsem_proto::gateway_grant::auth::{GatewayAuthDenial, GatewayAuthSecret};

pub(crate) struct Authority {
    operator: Mutex<Option<Operator>>,
    pub(crate) agent_mutation: tokio::sync::Mutex<()>,
    pub(crate) agents: capsem_core::agent_access::Registry,
}
impl Default for Authority {
    fn default() -> Self {
        Self {
            operator: Mutex::new(None),
            agent_mutation: tokio::sync::Mutex::new(()),
            agents: capsem_core::agent_access::Registry::new(
                capsem_proto::AGENT_ACCESS_MAX_ACTIVE,
                capsem_proto::AGENT_ACCESS_MAX_ISSUED_PER_EPOCH,
            )
            .expect("validated delegated runtime limits"),
        }
    }
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
        let candidate = refusal::bearer(headers)?;
        if candidate.contains('.') {
            let (id, capability) = refusal::delegated(candidate)?;
            let grant = self
                .agents
                .authenticate(id, &capability, || {
                    capsem_core::managed_sessions::LeaseClock::now().ok()
                })
                .map_err(|error| match error {
                    capsem_core::agent_access::Failure::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
                    _ => StatusCode::UNAUTHORIZED,
                })?;
            return Ok(AuthenticatedActor {
                evidence: capsem_api::CallerActor::DelegatedAgent {
                    access_id: id.to_string(),
                },
                agent: Some(grant),
                session: None,
            });
        }
        let operator = self.operator.lock().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let valid = operator.as_ref().is_some_and(|operator| {
            capsem_crypto::secrets::constant_time_eq(candidate.as_bytes(), operator.token.as_bytes())
        });
        drop(operator);
        if !valid {
            return Err(StatusCode::UNAUTHORIZED);
        }
        Ok(AuthenticatedActor {
            evidence: capsem_api::CallerActor::Operator,
            agent: None,
            session: None,
        })
    }
}

impl OperatorEpoch {
    pub(crate) fn activate(&mut self, token: GatewayAuthSecret<64>) -> Result<(), GatewayAuthDenial> {
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
pub(crate) struct AuthenticatedActor {
    evidence: capsem_api::CallerActor,
    agent: Option<Arc<capsem_core::agent_access::Grant>>,
    session: Option<session::SessionAdmission>,
}

impl AuthenticatedActor {
    pub(crate) fn vm_owner(owner: &crate::owner_connection::OwnerConnection) -> Self {
        Self {
            evidence: capsem_api::CallerActor::VmOwner {
                session_id: owner.id().to_owned(),
                owner_generation: owner.generation().to_string(),
            },
            agent: None,
            session: None,
        }
    }
    pub(crate) fn evidence(&self) -> &capsem_api::CallerActor {
        &self.evidence
    }
    pub(crate) fn require_operator(&self) -> Result<(), crate::AppError> {
        if !matches!(self.evidence, capsem_api::CallerActor::Operator) {
            return Err(crate::AppError::new(
                StatusCode::FORBIDDEN,
                "operator authority required".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn agent_grant(&self) -> Option<&Arc<capsem_core::agent_access::Grant>> {
        self.agent.as_ref()
    }
}
impl<S: Send + Sync> FromRequestParts<S> for AuthenticatedActor {
    type Rejection = StatusCode;
    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts.extensions.get::<Self>().cloned().ok_or(StatusCode::UNAUTHORIZED)
    }
}

mod refusal;
pub(crate) mod routing;
pub(crate) mod session;

#[cfg(test)]
mod tests;

/// Unit fixtures supply real credentials, never a production auth bypass.
#[cfg(test)]
pub(crate) fn fixture_authority() -> Arc<Authority> {
    Arc::new(Authority {
        operator: Mutex::new(Some(Operator {
            epoch: capsem_crypto::uuid_v4(),
            token: GatewayAuthSecret::from_bytes([b'A'; 64]),
        })),
        ..Authority::default()
    })
}
