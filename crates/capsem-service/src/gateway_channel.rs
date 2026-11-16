//! Service-owned HTTP seats for the confined gateway. A socket handed over
//! the inherited descriptor broker carries this proof; public UDS peers and
//! caller-controlled headers cannot construct it.
use std::{io, os::unix::net::UnixStream, sync::Arc};

use axum::{body::Body, extract::FromRequestParts, http::request::Parts, http::StatusCode};
use hyper_util::rt::TokioIo;
use tower::ServiceExt;

/// Sealed transport proof. Caller authentication is validated separately on
/// every request; a pooled gateway connection never conveys an actor alone.
#[derive(Clone, Copy)]
pub(crate) struct GatewayChannel(());

impl<S: Send + Sync> FromRequestParts<S> for GatewayChannel {
    type Rejection = StatusCode;
    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts.extensions.get::<Self>().copied().ok_or(StatusCode::FORBIDDEN)
    }
}

pub(crate) fn prepare(app: axum::Router, authority: Arc<crate::caller_auth::Authority>) -> io::Result<UnixStream> {
    let (client, server) = UnixStream::pair()?;
    client.set_nonblocking(true)?;
    server.set_nonblocking(true)?;
    let server = tokio::net::UnixStream::from_std(server)?;
    tokio::spawn(async move {
        let service = hyper::service::service_fn(move |request: hyper::Request<hyper::body::Incoming>| {
            let app = app.clone();
            let authority = Arc::clone(&authority);
            async move {
                let mut request = request.map(Body::new);
                if let Some(value) = request.headers_mut().get_mut(axum::http::header::AUTHORIZATION) {
                    value.set_sensitive(true);
                }
                let actor = match authority.authenticate_gateway(request.headers()) {
                    Ok(actor) => actor,
                    Err(status) => return Ok(axum::response::IntoResponse::into_response(status)),
                };
                tracing::debug!(actor = %actor.evidence().ledger_id(), "authenticated gateway request");
                request.extensions_mut().insert(GatewayChannel(()));
                request.extensions_mut().insert(actor);
                app.oneshot(request).await
            }
        });
        if let Err(error) = hyper::server::conn::http1::Builder::new()
            .serve_connection(TokioIo::new(server), service)
            .with_upgrades()
            .await
        {
            tracing::debug!(%error, "gateway service channel closed");
        }
    });
    Ok(client)
}
