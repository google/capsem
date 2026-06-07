use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use hyper_util::rt::TokioIo;
use tokio::net::UnixStream;

use crate::AppState;

/// Maximum request body size (10 MB). Prevents OOM from malicious oversized payloads.
const MAX_BODY_SIZE: usize = 10 * 1024 * 1024;

/// Default request timeout. Long enough for suspend (quiescence up to 10s +
/// pause/save up to 15s) and exec operations.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// Safety timeout for the background HTTP connection driver. Prevents orphaned
/// tasks if neither side closes the connection cleanly.
const CONN_DRIVER_TIMEOUT: Duration = Duration::from_secs(300);

/// Forward an allowlisted gateway route to capsem-service over UDS.
pub async fn handle_proxy(State(state): State<Arc<AppState>>, req: Request) -> Response {
    let request_id = gateway_request_id();
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let query_present = req.uri().query().is_some();
    let content_length = req
        .headers()
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok());
    let started = Instant::now();

    let span = tracing::info_span!(
        target: "capsem_gateway",
        "capsem.gateway.proxy",
        gateway_request_id = %request_id,
        method = %method,
        path = %path,
        query_present,
        content_length = ?content_length,
        uds_path = %state.uds_path.display(),
        status = tracing::field::Empty,
        latency_ms = tracing::field::Empty,
        error = tracing::field::Empty,
    );
    let _span_guard = span.enter();
    tracing::info!(
        target: "capsem_gateway",
        "gateway.proxy.start"
    );

    if let Some(content_length) = req.headers().get(axum::http::header::CONTENT_LENGTH) {
        if let Ok(len) = content_length.to_str().unwrap_or("").parse::<usize>() {
            if len > MAX_BODY_SIZE {
                span.record("status", StatusCode::PAYLOAD_TOO_LARGE.as_u16());
                span.record("latency_ms", started.elapsed().as_millis() as u64);
                tracing::warn!(
                    target: "capsem_gateway",
                    content_length = len,
                    max_body_size = MAX_BODY_SIZE,
                    "gateway.proxy.reject_oversized"
                );
                return (
                    StatusCode::PAYLOAD_TOO_LARGE,
                    axum::Json(serde_json::json!({"error": "request body too large"})),
                )
                    .into_response();
            }
        }
    }

    match forward(&state, req).await {
        Ok(resp) => {
            span.record("status", resp.status().as_u16());
            span.record("latency_ms", started.elapsed().as_millis() as u64);
            tracing::info!(
                target: "capsem_gateway",
                "gateway.proxy.ok"
            );
            resp
        }
        Err(e) => {
            span.record("status", StatusCode::BAD_GATEWAY.as_u16());
            span.record("latency_ms", started.elapsed().as_millis() as u64);
            span.record("error", tracing::field::display(&e));
            tracing::error!(
                target: "capsem_gateway",
                error = %e,
                "gateway.proxy.error"
            );
            (
                StatusCode::BAD_GATEWAY,
                axum::Json(serde_json::json!({"error": "service unavailable"})),
            )
                .into_response()
        }
    }
}

fn gateway_request_id() -> String {
    format!("{:012x}", rand::random::<u64>() & 0x0000_ffff_ffff_ffff)
}

async fn forward(state: &AppState, mut req: Request) -> anyhow::Result<Response> {
    let uri = req.uri().clone();

    // Clean up headers
    let headers = req.headers_mut();
    headers.remove(http::header::HOST);
    headers.remove(http::header::AUTHORIZATION);

    // Connect to UDS
    let stream = UnixStream::connect(&state.uds_path).await?;
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await?;
    tokio::spawn(async move {
        match tokio::time::timeout(CONN_DRIVER_TIMEOUT, conn).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::debug!(error = %e, "UDS connection driver error"),
            Err(_) => tracing::warn!("UDS connection driver timed out"),
        }
    });

    // Build upstream request preserving method, path, and query
    let upstream_uri = if let Some(q) = uri.query() {
        format!("http://localhost{}?{}", uri.path(), q)
    } else {
        format!("http://localhost{}", uri.path())
    };
    *req.uri_mut() = upstream_uri
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid upstream URI: {e}"))?;

    let (parts, body) = req.into_parts();

    // Wrap body in length limit for chunked requests
    use http_body_util::Limited;
    let limited_body = axum::body::Body::new(Limited::new(body, MAX_BODY_SIZE));
    let upstream_req = hyper::Request::from_parts(parts, limited_body);

    // Send with timeout
    let res = tokio::time::timeout(REQUEST_TIMEOUT, sender.send_request(upstream_req))
        .await
        .map_err(|_| anyhow::anyhow!("request timed out"))??;

    let (parts, body) = res.into_parts();
    Ok(Response::from_parts(parts, axum::body::Body::new(body)))
}

#[cfg(test)]
mod tests;
