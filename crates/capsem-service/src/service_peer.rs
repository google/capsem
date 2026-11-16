//! Kernel identity attached to every request accepted on the service UDS.

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
        return forbidden("foreign service peer denied");
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
            Err(_) => return forbidden("VM owner service identity denied"),
        };
        if !owner_route(owner.id(), request.uri().path()) {
            return forbidden("VM owner service route denied");
        }
        request
            .extensions_mut()
            .insert(crate::caller_auth::AuthenticatedActor::vm_owner(&owner));
        return next.run(request).await;
    }
    if request
        .extensions()
        .get::<crate::gateway_channel::GatewayChannel>()
        .is_some()
    {
        if let Some(value) = request.headers_mut().get_mut(axum::http::header::AUTHORIZATION) {
            value.set_sensitive(true);
        }
        let actor = match state.caller_authority.authenticate_gateway(request.headers()) {
            Ok(actor) => actor,
            Err(status) => return status.into_response(),
        };
        // Delegated routes must opt into an explicit operation/target scope.
        // No public issuance is enabled before those route policies are installed.
        if actor.agent_grant().is_some() {
            return forbidden("delegated service operation requires an authorized scope");
        }
        tracing::debug!(actor = %actor.evidence().ledger_id(), "authenticated service request");
        request.extensions_mut().insert(actor);
        return next.run(request).await;
    }
    if direct_status_route(request.method(), request.uri().path()) {
        return next.run(request).await;
    }
    forbidden("service API requires authenticated gateway transport")
}

fn forbidden(message: &'static str) -> axum::response::Response {
    axum::response::IntoResponse::into_response((
        axum::http::StatusCode::FORBIDDEN,
        axum::Json(serde_json::json!({"error": message})),
    ))
}

fn direct_status_route(method: &axum::http::Method, path: &str) -> bool {
    method == axum::http::Method::GET && matches!(path, "/version" | "/status" | "/system/status" | "/assets/status")
}

fn owner_route(owner: &str, path: &str) -> bool {
    path == "/networks/private/resolve" || path == format!("/internal/vms/{owner}/metrics")
}

#[cfg(test)]
mod tests;
