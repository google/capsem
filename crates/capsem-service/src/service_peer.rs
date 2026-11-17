//! Kernel identity attached to every request accepted on the service UDS.

use crate::agent_access::refusals::Reason;
use axum::extract::connect_info::Connected;
use axum::serve::IncomingStream;
use std::os::fd::AsFd;
use tokio::net::UnixListener;

#[derive(Clone, Copy, Debug)]
pub(crate) struct ServicePeer(pub(crate) Option<capsem_foundation::unix::peer::PeerIdentity>);

impl Connected<IncomingStream<'_, UnixListener>> for ServicePeer {
    fn connect_info(stream: IncomingStream<'_, UnixListener>) -> Self {
        Self(capsem_foundation::unix::peer::identity(stream.io().as_fd()).ok())
    }
}

/// The public UDS grants no ambient same-user API authority. The gateway's
/// inherited private seat authenticates each credential; registered VM owners
/// get only their two kernel-bound internal operations. Bootstrap/admin retain
/// named read-only status probes and cannot mutate through those exceptions.
pub(crate) async fn restrict_owner_routes(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<crate::ServiceState>>,
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::{extract::FromRequestParts, response::IntoResponse};
    let (mut parts, body) = request.into_parts();
    let peer = axum::extract::ConnectInfo::<ServicePeer>::from_request_parts(&mut parts, &())
        .await
        .ok()
        .and_then(|info| info.0 .0);
    request = axum::extract::Request::from_parts(parts, body);
    let current_uid = capsem_foundation::unix::process::current_uid();
    if peer.is_some_and(|peer| peer.uid != current_uid) {
        return refuse(&state, request, Reason::ForeignPeer, None).await;
    }
    let is_owner = peer.is_some_and(|peer| {
        state
            .instances
            .lock()
            .unwrap()
            .values()
            .any(|instance| instance.pid == peer.pid.get())
    });
    if is_owner {
        let owner = match crate::owner_connection::OwnerConnection::from_peer(&state, peer) {
            Ok(owner) => owner,
            Err(_) => return refuse(&state, request, Reason::OwnerIdentityDenied, None).await,
        };
        if !owner_route(owner.id(), request.uri().path()) {
            return refuse(
                &state,
                request,
                Reason::OwnerRouteDenied,
                Some(
                    crate::caller_auth::AuthenticatedActor::vm_owner(&owner)
                        .evidence()
                        .clone(),
                ),
            )
            .await;
        }
        request
            .extensions_mut()
            .insert(crate::caller_auth::AuthenticatedActor::vm_owner(&owner));
        return next.run(request).await;
    }
    if let Some(channel) = request.extensions().get::<crate::gateway_channel::GatewayChannel>() {
        if channel.is_revoked() {
            return refuse(&state, request, Reason::GatewayEpochEnded, None).await;
        }
        if let Some(value) = request.headers_mut().get_mut(axum::http::header::AUTHORIZATION) {
            value.set_sensitive(true);
        }
        let mut actor = match state.caller_authority.authenticate_gateway(request.headers()) {
            Ok(actor) => actor,
            Err(axum::http::StatusCode::UNAUTHORIZED) => {
                return refuse(&state, request, Reason::CredentialDenied, None).await
            }
            Err(status) => return status.into_response(),
        };
        // Delegated routes must opt into an explicit operation/target scope.
        if actor.agent_grant().is_some() {
            let exec = request.method() == axum::http::Method::POST
                && request
                    .extensions()
                    .get::<axum::extract::MatchedPath>()
                    .is_some_and(|path| path.as_str() == "/vms/{id}/exec");
            let admitted = exec
                && request
                    .uri()
                    .path()
                    .strip_prefix("/vms/")
                    .and_then(|path| path.strip_suffix("/exec"))
                    .is_some_and(|id| actor.bind_execution(&state, id));
            if !admitted {
                return refuse(&state, request, Reason::ScopeRequired, Some(actor.evidence().clone())).await;
            }
        }
        tracing::debug!(actor = %actor.evidence().ledger_id(), "authenticated service request");
        request.extensions_mut().insert(actor);
        return next.run(request).await;
    }
    if direct_status_route(request.method(), request.uri().path()) {
        return next.run(request).await;
    }
    refuse(&state, request, Reason::GatewayTransportRequired, None).await
}

async fn refuse(
    state: &crate::ServiceState,
    request: axum::extract::Request,
    reason: Reason,
    evidence: Option<capsem_api::CallerActor>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let evidence = match evidence {
        Some(evidence) => Some(evidence),
        None => match state.caller_authority.refusal_actor(request.headers()) {
            Ok(evidence) => evidence,
            Err(status) => return status.into_response(),
        },
    };
    if let Err(error) = crate::agent_access::refusals::record(state, evidence, request, reason).await {
        return error.into_response();
    }
    (
        reason.status(),
        axum::Json(serde_json::json!({"error": reason.message()})),
    )
        .into_response()
}

fn direct_status_route(method: &axum::http::Method, path: &str) -> bool {
    method == axum::http::Method::GET && matches!(path, "/version" | "/status" | "/system/status" | "/assets/status")
}

fn owner_route(owner: &str, path: &str) -> bool {
    path == "/networks/private/resolve" || path == format!("/internal/vms/{owner}/metrics")
}

#[cfg(test)]
mod tests;
