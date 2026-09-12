//! One HTTP request to the service over its Unix socket, from a process the
//! service spawned.
//!
//! The VM owner is a client of the service in exactly one place: asking for a
//! private connection on its VM's behalf. One request per call, no pool, no
//! keep-alive: the answer is small and the socket is local.
use anyhow::{Context, Result};
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use std::path::Path;
use std::time::Duration;

const REQUEST_DEADLINE: Duration = Duration::from_secs(12);

/// POST a JSON document and read back the status and JSON body.
pub async fn post_json(socket: &Path, path: &str, body: &serde_json::Value) -> Result<(u16, serde_json::Value)> {
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
        let request = http::Request::post(path)
            .header(http::header::HOST, "capsem")
            .header(http::header::CONTENT_TYPE, "application/json")
            .body(Full::new(Bytes::from(body.to_string())))
            .context("build service request")?;
        let response = sender.send_request(request).await.context("send service request")?;
        let status = response.status().as_u16();
        let bytes = response
            .into_body()
            .collect()
            .await
            .context("read service response")?
            .to_bytes();
        let value = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| serde_json::Value::String(String::from_utf8_lossy(&bytes).into_owned()))
        };
        Ok((status, value))
    })
    .await
    .context("service request deadline exceeded")?
}
