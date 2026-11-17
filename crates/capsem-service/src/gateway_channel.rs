//! Service-owned HTTP seats for the confined gateway. A socket handed over
//! the inherited descriptor broker carries this proof; public UDS peers and
//! caller-controlled headers cannot construct it.
use std::{io, os::unix::net::UnixStream};

use axum::{body::Body, extract::FromRequestParts, http::request::Parts, http::StatusCode};
use hyper_util::rt::TokioIo;
use tower::ServiceExt;

/// Sealed transport proof. Caller authentication is validated separately on
/// every request; a pooled gateway connection never conveys an actor alone.
#[derive(Clone)]
pub(crate) struct GatewayChannel(crate::instance::WorkerGrant);

impl GatewayChannel {
    pub(crate) fn is_revoked(&self) -> bool {
        self.0.is_revoked()
    }

    pub(crate) async fn revoked(&self) {
        self.0.revoked().await;
    }
}

impl<S: Send + Sync> FromRequestParts<S> for GatewayChannel {
    type Rejection = StatusCode;
    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let channel = parts.extensions.get::<Self>().cloned().ok_or(StatusCode::FORBIDDEN)?;
        if channel.is_revoked() {
            return Err(StatusCode::FORBIDDEN);
        }
        Ok(channel)
    }
}

pub(crate) fn prepare(app: axum::Router, control: crate::instance::WorkerGrant) -> io::Result<UnixStream> {
    let (client, server) = UnixStream::pair()?;
    client.set_nonblocking(true)?;
    server.set_nonblocking(true)?;
    let server = tokio::net::UnixStream::from_std(server)?;
    tokio::spawn(async move {
        let channel = GatewayChannel(control.clone());
        let service = hyper::service::service_fn(move |request: hyper::Request<hyper::body::Incoming>| {
            let app = app.clone();
            let channel = channel.clone();
            async move {
                let mut request = request.map(Body::new);
                request.extensions_mut().insert(channel);
                app.oneshot(request).await
            }
        });
        let http = hyper::server::conn::http1::Builder::new();
        tokio::select! {
            result = http.serve_connection(TokioIo::new(server), service).with_upgrades() => {
                if let Err(error) = result {
                    tracing::debug!(%error, "gateway service channel closed");
                }
            }
            () = control.revoked() => {}
        }
    });
    Ok(client)
}
