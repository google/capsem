//! One HTTP request to the service over its Unix socket, from a process the
//! service spawned.
//!
//! Spawned workers use fixed internal service routes for brokered operations.
//! One request uses one connection, with no pool or keep-alive: exchanges are
//! bounded and the socket is local.
use anyhow::{ensure, Context, Result};
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Bytes;
use hyper::client::conn::http1::SendRequest;
use hyper_util::rt::TokioIo;
use std::path::Path;
use std::time::Duration;
use tokio::net::UnixStream;

const REQUEST_DEADLINE: Duration = Duration::from_secs(12);
/// Maximum request or response body carried over the worker-to-service socket.
pub const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;

/// A persistent service connection prepared before a worker drops path and
/// connect authority. Requests are serialized because HTTP/1 has one ordered
/// response stream; response bodies are collected before the next request.
pub struct Client {
    sender: tokio::sync::Mutex<SendRequest<Full<Bytes>>>,
}

impl Client {
    pub async fn connect(socket: &Path) -> Result<Self> {
        let stream = UnixStream::connect(socket)
            .await
            .with_context(|| format!("connect service socket {}", socket.display()))?;
        Self::from_stream(stream).await
    }

    pub async fn from_stream(stream: UnixStream) -> Result<Self> {
        let (sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
            .await
            .context("service HTTP handshake")?;
        tokio::spawn(async move {
            if let Err(error) = connection.await {
                tracing::debug!(%error, "service connection ended");
            }
        });
        Ok(Self {
            sender: tokio::sync::Mutex::new(sender),
        })
    }

    pub async fn post_json(&self, path: &str, body: &serde_json::Value) -> Result<(u16, serde_json::Value)> {
        let response = self
            .post_bytes(
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

    pub async fn post_bytes(
        &self,
        path: &str,
        content_type: http::HeaderValue,
        body: Bytes,
    ) -> Result<http::Response<Bytes>> {
        let request = request(path, content_type, body)?;
        self.send(request).await
    }

    async fn send(&self, request: http::Request<Full<Bytes>>) -> Result<http::Response<Bytes>> {
        tokio::time::timeout(REQUEST_DEADLINE, async {
            let mut sender = self.sender.lock().await;
            let response = sender.send_request(request).await.context("send service request")?;
            let (parts, body) = response.into_parts();
            let bytes = Limited::new(body, MAX_BODY_BYTES)
                .collect()
                .await
                .map_err(|error| anyhow::anyhow!("service response body exceeds limit or could not be read: {error}"))?
                .to_bytes();
            drop(sender);
            Ok(http::Response::from_parts(parts, bytes))
        })
        .await
        .context("service request deadline exceeded")?
    }
}

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
    let request = request(path, content_type, body)?;
    Client::connect(socket).await?.send(request).await
}

fn request(path: &str, content_type: http::HeaderValue, body: Bytes) -> Result<http::Request<Full<Bytes>>> {
    ensure!(
        body.len() <= MAX_BODY_BYTES,
        "service request body exceeds {MAX_BODY_BYTES} bytes"
    );
    http::Request::post(path)
        .header(http::header::HOST, "capsem")
        .header(http::header::CONTENT_TYPE, content_type)
        .body(Full::new(body))
        .context("build service request")
}

#[cfg(test)]
mod tests;
