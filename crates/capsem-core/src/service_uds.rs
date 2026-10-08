//! One HTTP request to the service over its Unix socket, from a process the
//! service spawned.
//!
//! Spawned workers use fixed internal service routes for brokered operations.
//! One request uses one connection, with no pool or keep-alive: exchanges are
//! bounded and the socket is local.
use anyhow::{ensure, Context, Result};
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use std::path::Path;
use std::time::Duration;

const REQUEST_DEADLINE: Duration = Duration::from_secs(12);
/// Maximum request or response body carried over the worker-to-service socket.
pub const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;

/// POST a JSON document and read back the status and JSON body.
pub async fn post_json(socket: &Path, path: &str, body: &serde_json::Value) -> Result<(u16, serde_json::Value)> {
    let response = post_bytes(
        socket,
        path,
        http::HeaderValue::from_static("application/json"),
        Bytes::from(body.to_string()),
    )
    .await?;
    let status = response.status().as_u16();
    let bytes = response.into_body();
    let value = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| serde_json::Value::String(String::from_utf8_lossy(&bytes).into_owned()))
    };
    Ok((status, value))
}

/// POST one bounded byte body and return the bounded response from the service.
///
/// The path and content type are explicit because callers use fixed internal
/// routes. One connection is used for one request, and the deadline covers
/// connect, write, response headers, and response body collection.
pub async fn post_bytes(
    socket: &Path,
    path: &str,
    content_type: http::HeaderValue,
    body: Bytes,
) -> Result<http::Response<Bytes>> {
    ensure!(
        body.len() <= MAX_BODY_BYTES,
        "service request body exceeds {MAX_BODY_BYTES} bytes"
    );
    let request = http::Request::post(path)
        .header(http::header::HOST, "capsem")
        .header(http::header::CONTENT_TYPE, content_type)
        .body(Full::new(body))
        .context("build service request")?;
    tokio::time::timeout(REQUEST_DEADLINE, async {
        let stream = tokio::net::UnixStream::connect(socket)
            .await
            .with_context(|| format!("connect service socket {}", socket.display()))?;
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
            .await
            .context("service HTTP handshake")?;
        tokio::spawn(async move {
            if let Err(error) = connection.await {
                tracing::debug!(%error, "service connection ended");
            }
        });
        let response = sender.send_request(request).await.context("send service request")?;
        let (parts, body) = response.into_parts();
        let bytes = Limited::new(body, MAX_BODY_BYTES)
            .collect()
            .await
            .map_err(|error| anyhow::anyhow!("service response body exceeds limit or could not be read: {error}"))?
            .to_bytes();
        Ok(http::Response::from_parts(parts, bytes))
    })
    .await
    .context("service request deadline exceeded")?
}

#[cfg(test)]
mod tests;
