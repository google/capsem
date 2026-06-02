use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use hyper_util::rt::TokioIo;
use tokio::net::UnixStream;

use crate::metrics as gateway_metrics;
use crate::AppState;

/// Maximum request body size (10 MB). Prevents OOM from malicious oversized payloads.
const MAX_BODY_SIZE: usize = 10 * 1024 * 1024;

/// Default request timeout. Long enough for suspend (quiescence up to 10s +
/// pause/save up to 15s) and exec operations.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// Safety timeout for the background HTTP connection driver. Prevents orphaned
/// tasks if neither side closes the connection cleanly.
const CONN_DRIVER_TIMEOUT: Duration = Duration::from_secs(300);

/// Catch-all handler: forward any request to capsem-service over UDS.
pub async fn handle_proxy(State(state): State<Arc<AppState>>, req: Request) -> Response {
    gateway_metrics::describe_all();
    let started = std::time::Instant::now();
    let method = method_label(req.method());
    let endpoint = endpoint_class(req.uri().path());
    if let Some(content_length) = req.headers().get(axum::http::header::CONTENT_LENGTH) {
        if let Ok(len) = content_length.to_str().unwrap_or("").parse::<usize>() {
            if len > MAX_BODY_SIZE {
                record_proxy_request(started, method, endpoint, "4xx");
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
            let status = status_class(resp.status());
            record_proxy_request(started, method, endpoint, status);
            resp
        }
        Err(e) => {
            tracing::error!(error = %e, "proxy error");
            record_proxy_request(started, method, endpoint, "5xx");
            (
                StatusCode::BAD_GATEWAY,
                axum::Json(serde_json::json!({"error": "service unavailable"})),
            )
                .into_response()
        }
    }
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

fn record_proxy_request(
    started: std::time::Instant,
    method: &'static str,
    endpoint: &'static str,
    status_class: &'static str,
) {
    ::metrics::counter!(
        gateway_metrics::PROXY_REQUESTS_TOTAL,
        "endpoint" => endpoint,
        "method" => method,
        "status_class" => status_class,
    )
    .increment(1);
    ::metrics::histogram!(
        gateway_metrics::PROXY_REQUEST_DURATION_MS,
        "endpoint" => endpoint,
        "method" => method,
        "status_class" => status_class,
    )
    .record(started.elapsed().as_secs_f64() * 1000.0);
}

fn method_label(method: &http::Method) -> &'static str {
    match *method {
        http::Method::GET => "GET",
        http::Method::POST => "POST",
        http::Method::PUT => "PUT",
        http::Method::PATCH => "PATCH",
        http::Method::DELETE => "DELETE",
        http::Method::HEAD => "HEAD",
        _ => "OTHER",
    }
}

fn status_class(status: StatusCode) -> &'static str {
    match status.as_u16() {
        100..=199 => "1xx",
        200..=299 => "2xx",
        300..=399 => "3xx",
        400..=499 => "4xx",
        500..=599 => "5xx",
        _ => "other",
    }
}

fn endpoint_class(path: &str) -> &'static str {
    match path.trim_start_matches('/').split('/').next().unwrap_or("") {
        "" => "root",
        "confirm" => "confirm",
        "credentials" => "credentials",
        "debug" => "debug",
        "delete" => "delete",
        "detection" => "detection",
        "enforcement" => "enforcement",
        "events" => "events",
        "exec" => "exec",
        "files" => "files",
        "fork" => "fork",
        "health" => "health",
        "history" => "history",
        "host-logs" => "host_logs",
        "info" => "info",
        "inspect" => "inspect",
        "list" => "list",
        "logs" => "logs",
        "mcp" => "mcp",
        "panics" => "panics",
        "persist" => "persist",
        "profiles" => "profiles",
        "provision" => "provision",
        "purge" => "purge",
        "resume" => "resume",
        "rules" => "rules",
        "run" => "run",
        "service-logs" => "service_logs",
        "settings" => "settings",
        "setup" => "setup",
        "skills" => "skills",
        "stats" => "stats",
        "stop" => "stop",
        "suspend" => "suspend",
        "terminal" => "terminal",
        "timeline" => "timeline",
        "triage" => "triage",
        _ => "other",
    }
}

#[cfg(test)]
mod tests;
