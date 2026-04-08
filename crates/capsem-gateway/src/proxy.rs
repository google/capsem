use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper_util::rt::TokioIo;
use tokio::net::UnixStream;

use crate::AppState;

/// Catch-all handler: forward any request to capsem-service over UDS.
pub async fn handle_proxy(
    State(state): State<Arc<AppState>>,
    req: Request,
) -> Response {
    match forward(&state, req).await {
        Ok(resp) => resp,
        Err(e) => {
            tracing::error!(error = %e, "proxy error");
            (
                StatusCode::BAD_GATEWAY,
                axum::Json(serde_json::json!({"error": "service unavailable"})),
            )
                .into_response()
        }
    }
}

async fn forward(state: &AppState, req: Request) -> anyhow::Result<Response> {
    let method = req.method().clone();
    let uri = req.uri().clone();
    let content_type = req
        .headers()
        .get("content-type")
        .cloned();

    // Collect incoming body
    let body_bytes = req
        .into_body()
        .collect()
        .await
        .map(|c| c.to_bytes())
        .unwrap_or_default();

    // Connect to UDS
    let stream = UnixStream::connect(&state.uds_path).await?;
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await?;
    tokio::spawn(async move {
        if let Err(e) = conn.await {
            tracing::error!(error = %e, "UDS connection error");
        }
    });

    // Build upstream request preserving method, path, and query
    let upstream_uri = if let Some(q) = uri.query() {
        format!("http://localhost{}?{}", uri.path(), q)
    } else {
        format!("http://localhost{}", uri.path())
    };

    let mut builder = hyper::Request::builder()
        .method(method)
        .uri(upstream_uri);

    if let Some(ct) = content_type {
        builder = builder.header("content-type", ct);
    }

    let upstream_req = builder.body(Full::new(body_bytes))?;

    // Send with timeout
    let res = tokio::time::timeout(Duration::from_secs(30), sender.send_request(upstream_req))
        .await
        .map_err(|_| anyhow::anyhow!("request timed out"))??;

    // Convert hyper response to axum response
    let status = res.status();
    let headers = res.headers().clone();
    let body_bytes: Bytes = res.into_body().collect().await?.to_bytes();

    let mut response = Response::builder().status(status);
    for (key, value) in headers.iter() {
        response = response.header(key, value);
    }

    Ok(response
        .body(Body::from(body_bytes))
        .unwrap_or_else(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal error",
            )
                .into_response()
        }))
}
