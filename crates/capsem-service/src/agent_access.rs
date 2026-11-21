//! Operator issuance is durable before activation; history never restores a grant.
use std::{
    sync::Arc,
    time::{Duration, UNIX_EPOCH},
};

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use capsem_api::{AgentAccessScope, AgentAccessSummary, CreateAgentAccessRequest, CreateAgentAccessResponse};
use capsem_core::{
    agent_access::{Failure, Grant, Reservation},
    managed_sessions::{Capability, LeaseClock},
};
use capsem_logger::{HostEvent, HostEventKind};
use serde::{Deserialize, Serialize};

use crate::{caller_auth::AuthenticatedActor, AppError, ServiceState};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Issued {
    access_id: String,
    epoch: String,
    scope: AgentAccessScope,
    lease_seconds: u32,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Revoked {
    access_id: String,
    epoch: String,
}

fn failure(error: Failure) -> AppError {
    let status = match error {
        Failure::Forbidden => StatusCode::FORBIDDEN,
        Failure::Pending | Failure::Closed | Failure::Conflict => StatusCode::CONFLICT,
        Failure::Capacity => StatusCode::TOO_MANY_REQUESTS,
        Failure::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        Failure::Invalid => StatusCode::BAD_REQUEST,
        Failure::NotFound => StatusCode::NOT_FOUND,
    };
    AppError::new(status, error.to_string())
}
fn unavailable(error: impl std::fmt::Display) -> AppError {
    tracing::error!(%error, "delegated access audit unavailable");
    AppError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "delegated access audit unavailable".into(),
    )
}
fn identity(value: &str) -> Result<uuid::Uuid, AppError> {
    capsem_api::validate_managed_request_id(value).map_err(|_| failure(Failure::Invalid))?;
    uuid::Uuid::parse_str(value).map_err(|_| failure(Failure::Invalid))
}
fn summary(grant: &Grant) -> Result<CreateAgentAccessResponse, AppError> {
    let expiry = UNIX_EPOCH
        .checked_add(Duration::from_millis(grant.expires_wall_ms()))
        .ok_or_else(|| failure(Failure::Unavailable))?;
    Ok(CreateAgentAccessResponse {
        access: AgentAccessSummary {
            id: grant.id().to_string(),
            scope: grant.scope().clone(),
            expires_at: capsem_foundation::time::format_rfc3339_millis(expiry),
        },
    })
}

/// Query in bounded pages through the logger owner, never a service projection.
async fn historically_issued(state: &ServiceState, id: &str) -> Result<bool, AppError> {
    let mut cursor = 0;
    loop {
        let page = state
            .host_ledger
            .host_event_page(
                HostEventKind::GrantMinted,
                cursor,
                capsem_logger::ledger_protocol::MAX_LEDGER_QUERY_ROWS,
            )
            .await
            .map_err(unavailable)?;
        if page.is_empty() {
            return Ok(false);
        }
        for (row_id, event) in page {
            let issued: Issued = rmp_serde::from_slice(&event.detail).map_err(unavailable)?;
            if issued.access_id == id {
                return Ok(true);
            }
            cursor = row_id;
        }
    }
}

async fn audit(state: &ServiceState, actor: String, kind: HostEventKind, detail: Vec<u8>) -> Result<(), AppError> {
    let elapsed = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(unavailable)?;
    let timestamp = i64::try_from(elapsed.as_millis()).map_err(unavailable)?;
    state
        .record_host_event(HostEvent {
            timestamp_unix_ms: timestamp,
            kind,
            session_id: None,
            actor,
            detail,
            trace_id: None,
        })
        .await
        .map_err(unavailable)
}

pub(crate) async fn issue(
    actor: AuthenticatedActor,
    State(state): State<Arc<ServiceState>>,
    Json(request): Json<CreateAgentAccessRequest>,
) -> Result<Json<CreateAgentAccessResponse>, AppError> {
    actor.require_operator()?;
    let id = identity(&request.request_id)?;
    let capability = Capability::from_bytes(request.ownership_token.to_bytes());
    let _mutation = state.caller_authority.agent_mutation.lock().await;
    let reservation = state
        .caller_authority
        .agents
        .reserve(
            id,
            &capability,
            request.scope.clone(),
            Duration::from_secs(u64::from(request.lease_seconds)),
            || LeaseClock::now().ok(),
        )
        .map_err(failure)?;
    match reservation {
        Reservation::Existing(grant) => Ok(Json(summary(&grant)?)),
        Reservation::New(admission) => {
            if historically_issued(&state, &request.request_id).await? {
                return Err(failure(Failure::Closed));
            }
            let detail = rmp_serde::to_vec_named(&Issued {
                access_id: request.request_id,
                epoch: state.caller_authority.agents.epoch().to_string(),
                scope: request.scope,
                lease_seconds: request.lease_seconds,
            })
            .map_err(unavailable)?;
            audit(&state, actor.evidence().ledger_id(), HostEventKind::GrantMinted, detail).await?;
            let grant = admission.commit(|| LeaseClock::now().ok()).map_err(failure)?;
            Ok(Json(summary(&grant)?))
        }
    }
}

pub(crate) async fn revoke(
    actor: AuthenticatedActor,
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<capsem_api::VmActionResponse>, AppError> {
    actor.require_operator()?;
    let parsed = identity(&id)?;
    let _mutation = state.caller_authority.agent_mutation.lock().await;
    match state.caller_authority.agents.revoke(parsed) {
        Ok(_) => {}
        Err(Failure::NotFound) if historically_issued(&state, &id).await? => {}
        Err(error) => return Err(failure(error)),
    }
    // No await occurs before live authority is cancelled. Failed audit never
    // reinstates the credential; the operator can safely retry this DELETE.
    let detail = rmp_serde::to_vec_named(&Revoked {
        access_id: id,
        epoch: state.caller_authority.agents.epoch().to_string(),
    })
    .map_err(unavailable)?;
    audit(
        &state,
        actor.evidence().ledger_id(),
        HostEventKind::GrantRevoked,
        detail,
    )
    .await?;
    Ok(Json(capsem_api::VmActionResponse { success: true }))
}

pub(crate) mod refusals;

#[cfg(test)]
mod tests;
