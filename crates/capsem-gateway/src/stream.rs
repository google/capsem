//! `GET /vms/{id}/stream`: authenticate, then tunnel the WebSocket upgrade to
//! capsem-service unchanged. The gateway neither parses nor buffers frames;
//! the service owns the stream protocol and its limits.

use super::*;
use axum::extract::{Path, Request, State};
use axum::response::Response;
use http::header::{
    CONNECTION, SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_KEY, SEC_WEBSOCKET_PROTOCOL, SEC_WEBSOCKET_VERSION, UPGRADE,
};
use hyper_util::rt::TokioIo;

/// Handshake headers carried to the service and back; everything else,
/// including the client's credentials, stays at the gateway.
const REQUEST_HEADERS: [http::HeaderName; 5] = [
    CONNECTION,
    UPGRADE,
    SEC_WEBSOCKET_KEY,
    SEC_WEBSOCKET_VERSION,
    SEC_WEBSOCKET_PROTOCOL,
];
const RESPONSE_HEADERS: [http::HeaderName; 4] = [CONNECTION, UPGRADE, SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_PROTOCOL];

/// Validate VM ID: alphanumeric, hyphens, underscores. Must start with
/// alphanumeric, length 1-64. Matches capsem-service's `validate_vm_name`.
pub(crate) fn validate_vm_id(id: &str) -> Result<(), &'static str> {
    if id.is_empty() {
        return Err("VM id cannot be empty");
    }
    if id.len() > 64 {
        return Err("VM id too long (max 64 characters)");
    }
    if !id.chars().next().unwrap().is_ascii_alphanumeric() {
        return Err("VM id must start with a letter or digit");
    }
    if !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err("VM id must contain only letters, digits, hyphens, and underscores");
    }
    Ok(())
}

fn refuse(status: http::StatusCode, message: &str) -> Response {
    (status, axum::Json(serde_json::json!({ "error": message }))).into_response()
}

pub async fn handle_stream_tunnel(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    mut request: Request,
) -> Response {
    if let Err(message) = validate_vm_id(&id) {
        return refuse(http::StatusCode::BAD_REQUEST, message);
    }
    let is_upgrade = request
        .headers()
        .get(UPGRADE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("websocket"));
    if !is_upgrade {
        return refuse(
            http::StatusCode::UPGRADE_REQUIRED,
            "stream requires a WebSocket upgrade",
        );
    }

    // The query may carry the gateway token for browsers; it is never forwarded.
    // An upgrade handshake is origin-form with a Host header (RFC 6455 4.1).
    let mut upstream = http::Request::builder()
        .method(http::Method::GET)
        .uri(format!("/vms/{id}/stream"))
        .header(http::header::HOST, "localhost");
    for name in REQUEST_HEADERS {
        for value in request.headers().get_all(&name) {
            upstream = upstream.header(&name, value);
        }
    }
    let Ok(upstream) = upstream.body(http_body_util::Empty::<bytes::Bytes>::new()) else {
        return refuse(http::StatusCode::BAD_REQUEST, "invalid stream handshake headers");
    };
    let client_upgrade = hyper::upgrade::on(&mut request);

    let socket = match tokio::net::UnixStream::connect(state.uds_path.as_path()).await {
        Ok(socket) => socket,
        Err(error) => {
            tracing::warn!(%id, %error, "stream tunnel: service unavailable");
            return refuse(http::StatusCode::BAD_GATEWAY, "service unavailable");
        }
    };
    let (mut sender, connection) = match hyper::client::conn::http1::handshake(TokioIo::new(socket)).await {
        Ok(pair) => pair,
        Err(error) => {
            tracing::warn!(%id, %error, "stream tunnel: service handshake failed");
            return refuse(http::StatusCode::BAD_GATEWAY, "service unavailable");
        }
    };
    tokio::spawn(connection.with_upgrades());
    let mut response = match sender.send_request(upstream).await {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!(%id, %error, "stream tunnel: service request failed");
            return refuse(http::StatusCode::BAD_GATEWAY, "service unavailable");
        }
    };
    if response.status() != http::StatusCode::SWITCHING_PROTOCOLS {
        // The service refused (unknown VM, missing subprotocol): relay its answer.
        let (parts, body) = response.into_parts();
        return Response::from_parts(parts, axum::body::Body::new(body));
    }

    let mut switching = Response::builder().status(http::StatusCode::SWITCHING_PROTOCOLS);
    for name in RESPONSE_HEADERS {
        for value in response.headers().get_all(&name) {
            switching = switching.header(&name, value);
        }
    }
    let upstream_upgrade = hyper::upgrade::on(&mut response);
    tokio::spawn(async move {
        match tokio::try_join!(client_upgrade, upstream_upgrade) {
            Ok((client, service)) => {
                let (mut client, mut service) = (TokioIo::new(client), TokioIo::new(service));
                if let Err(error) = tokio::io::copy_bidirectional(&mut client, &mut service).await {
                    tracing::debug!(%id, %error, "stream tunnel closed");
                }
            }
            Err(error) => tracing::warn!(%id, %error, "stream tunnel upgrade failed"),
        }
    });
    switching
        .body(axum::body::Body::empty())
        .unwrap_or_else(|_| refuse(http::StatusCode::BAD_GATEWAY, "invalid upgrade"))
}

#[cfg(test)]
mod tests;
