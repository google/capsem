use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Request, State};
use axum::http::{
    header::{CONTENT_LENGTH, CONTENT_TYPE, TRANSFER_ENCODING},
    uri::{Authority, Scheme},
    HeaderMap, HeaderName, HeaderValue, Method, StatusCode,
};
use axum::response::{IntoResponse, Response};
use http_body_util::BodyExt;

use crate::AppState;

/// Maximum request body size (10 MB). Prevents OOM from malicious oversized payloads.
const MAX_BODY_SIZE: usize = 10 * 1024 * 1024;

/// Default request timeout. Long enough for suspend (quiescence up to 10s +
/// pause/save up to 15s) and exec operations.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

const HOP_BY_HOP_REQUEST_HEADERS: [&str; 10] = [
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "http2-settings",
];

/// Forward an allowlisted gateway route to capsem-service over UDS.
pub async fn handle_proxy(State(state): State<Arc<AppState>>, req: Request) -> Response {
    let request_id = gateway_request_id();
    let query_present = req.uri().query().is_some();
    let content_length = req
        .headers()
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok());
    let started = Instant::now();

    let span = {
        let method = req.method();
        let path = req.uri().path();
        tracing::info_span!(
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
        )
    };
    let _span_guard = span.enter();
    tracing::debug!(
        target: "capsem_gateway",
        "gateway.proxy.start"
    );

    if content_length.is_some_and(|len| len > MAX_BODY_SIZE) {
        span.record("status", StatusCode::PAYLOAD_TOO_LARGE.as_u16());
        span.record("latency_ms", started.elapsed().as_millis() as u64);
        tracing::warn!(
            target: "capsem_gateway",
            content_length,
            max_body_size = MAX_BODY_SIZE,
            "gateway.proxy.reject_oversized"
        );
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            axum::Json(serde_json::json!({"error": "request body too large"})),
        )
            .into_response();
    }

    match forward(&state, req).await {
        Ok(resp) => {
            span.record("status", resp.status().as_u16());
            span.record("latency_ms", started.elapsed().as_millis() as u64);
            tracing::debug!(
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

struct GatewayRequestId(u64);

impl fmt::Display for GatewayRequestId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:012x}", self.0)
    }
}

fn gateway_request_id() -> GatewayRequestId {
    GatewayRequestId(rand::random::<u64>() & 0x0000_ffff_ffff_ffff)
}

async fn forward(state: &AppState, mut req: Request) -> anyhow::Result<Response> {
    let should_buffer_json = req.method() == Method::GET && {
        let path = req.uri().path();
        !path.contains("/logs") && !path.starts_with("/host-logs/") && path != "/service-logs"
    };

    // Clean up headers
    let headers = req.headers_mut();
    headers.remove(http::header::HOST);
    headers.remove(http::header::AUTHORIZATION);
    strip_hop_by_hop_request_headers(headers);

    // Build the absolute upstream URI while retaining the parsed path/query.
    let mut upstream_uri = req.uri().clone().into_parts();
    upstream_uri.scheme = Some(Scheme::HTTP);
    upstream_uri.authority = Some(Authority::from_static("localhost"));
    *req.uri_mut() =
        http::Uri::from_parts(upstream_uri).map_err(|error| anyhow::anyhow!("invalid upstream URI: {error}"))?;

    let (parts, body) = req.into_parts();

    // Wrap body in length limit for chunked requests
    use http_body_util::Limited;
    let limited_body = axum::body::Body::new(Limited::new(body, MAX_BODY_SIZE));
    let upstream_req = hyper::Request::from_parts(parts, limited_body);

    // Send with timeout
    let res = tokio::time::timeout(REQUEST_TIMEOUT, state.service_client.request(upstream_req))
        .await
        .map_err(|_| anyhow::anyhow!("request timed out"))??;

    let (mut parts, body) = res.into_parts();
    if should_buffer_json_response(should_buffer_json, &parts.headers) {
        let body = body.collect().await?.to_bytes();
        parts.headers.remove(TRANSFER_ENCODING);
        parts
            .headers
            .insert(CONTENT_LENGTH, HeaderValue::from_str(&body.len().to_string())?);
        return Ok(Response::from_parts(parts, axum::body::Body::from(body)));
    }
    Ok(Response::from_parts(parts, axum::body::Body::new(body)))
}

fn strip_hop_by_hop_request_headers(headers: &mut HeaderMap) {
    if headers.contains_key(http::header::CONNECTION) {
        let connection_headers = headers
            .get_all(http::header::CONNECTION)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(|value| value.split(','))
            .filter_map(|name| HeaderName::from_bytes(name.trim().as_bytes()).ok())
            .collect::<Vec<_>>();

        for name in connection_headers {
            headers.remove(name);
        }
    }
    for name in HOP_BY_HOP_REQUEST_HEADERS {
        headers.remove(name);
    }
}

fn should_buffer_json_response(candidate: bool, headers: &HeaderMap) -> bool {
    if !candidate || headers.contains_key(CONTENT_LENGTH) {
        return false;
    }
    headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("application/json"))
}

#[cfg(test)]
mod tests;
