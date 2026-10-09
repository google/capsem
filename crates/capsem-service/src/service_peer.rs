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

/// A registered VM owner may reach only the two service routes it needs after
/// startup. Route handlers still validate the VM ID and generation; this
/// outer boundary prevents a compromised owner from using the public API as a
/// same-user coordinator client.
pub(crate) async fn restrict_owner_routes(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<crate::ServiceState>>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let Some(peer) = request
        .extensions()
        .get::<axum::extract::ConnectInfo<ServicePeer>>()
        .and_then(|info| info.0 .0)
    else {
        return next.run(request).await;
    };
    let owner = state
        .instances
        .lock()
        .unwrap()
        .values()
        .find(|instance| instance.pid == peer.pid.get() && peer.uid == capsem_foundation::unix::process::current_uid())
        .map(|instance| instance.id.clone());
    let Some(owner) = owner else {
        return next.run(request).await;
    };
    if owner_route(&owner, request.uri().path()) {
        next.run(request).await
    } else {
        axum::response::IntoResponse::into_response((
            axum::http::StatusCode::FORBIDDEN,
            axum::Json(serde_json::json!({"error": "VM owner service route denied"})),
        ))
    }
}

fn owner_route(owner: &str, path: &str) -> bool {
    path == "/networks/private/resolve" || path == format!("/internal/vms/{owner}/metrics")
}

#[cfg(test)]
mod tests;
