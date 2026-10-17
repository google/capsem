//! HTTP over a connected, coordinator-scoped capability; never an ambient dial.

use bytes::Bytes;
use capsem_foundation::unix::fd;
use futures::StreamExt;
use http_body_util::BodyExt;
use hyper::client::conn::http2;
use hyper_util::rt::{TokioExecutor, TokioIo};
use reqwest::ResponseBuilderExt;
use std::io;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::time::Duration;

struct ConnectionTask(tokio::task::JoinHandle<()>);

impl Drop for ConnectionTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Multiplexed HTTP over one coordinator-supplied channel. Long-lived MCP SSE
/// bodies never serialize unrelated calls. The coordinator, not this client,
/// owns destination selection, credentials and the generation's revocation.
#[derive(Clone)]
pub struct BrokerHttpClient {
    sender: http2::SendRequest<reqwest::Body>,
    _connection: Arc<ConnectionTask>,
    timeout: Duration,
    capacity: Arc<tokio::sync::Semaphore>,
}

impl BrokerHttpClient {
    pub async fn connect(socket: UnixStream, timeout: Duration) -> io::Result<Self> {
        fd::validate_connected_stream(socket.as_fd())?;
        fd::set_nonblocking(socket.as_fd(), true)?;
        let stream = tokio::net::UnixStream::from_std(socket)?;
        let (sender, connection) = tokio::time::timeout(
            timeout,
            http2::Builder::new(TokioExecutor::new())
                .max_header_list_size(capsem_proto::MCP_FRAME_MAX_SIZE as u32)
                .handshake(TokioIo::new(stream)),
        )
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "MCP HTTP capability handshake timed out"))?
        .map_err(io::Error::other)?;
        let connection = tokio::spawn(async move {
            if let Err(error) = connection.await {
                tracing::debug!(%error, "MCP HTTP capability closed");
            }
        });
        Ok(Self {
            sender,
            _connection: Arc::new(ConnectionTask(connection)),
            timeout,
            capacity: Arc::new(tokio::sync::Semaphore::new(capsem_proto::NETWORK_CLIENT_CAPACITY)),
        })
    }

    /// Send once, with a finite response-header deadline. The response body
    /// streams with HTTP/2 flow control; dropping it cancels only that stream.
    pub async fn execute(&self, mut request: reqwest::Request) -> io::Result<reqwest::Response> {
        if let Some(body) = request.body() {
            let bytes = body.as_bytes().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "MCP worker request body must be buffered")
            })?;
            if bytes.len() > capsem_proto::MCP_FRAME_MAX_SIZE {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "MCP worker request body is too large",
                ));
            }
        }
        // Move the already-buffered body rather than copying every tool call.
        let body = request.body_mut().take().unwrap_or_else(|| Bytes::new().into());
        let timeout = request.timeout().copied().unwrap_or(self.timeout);
        let mut wire = http::Request::builder()
            .method(request.method().clone())
            .uri(request.url().as_str())
            .body(body)
            .map_err(io::Error::other)?;
        *wire.headers_mut() = request.headers().clone();
        let mut sender = self.sender.clone();
        let (response, permit) = tokio::time::timeout(timeout, async {
            let permit = self.capacity.clone().acquire_owned().await.map_err(io::Error::other)?;
            sender.ready().await.map_err(io::Error::other)?;
            let response = sender.send_request(wire).await.map_err(io::Error::other)?;
            Ok::<_, io::Error>((response, permit))
        })
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "MCP HTTP capability request timed out"))??;
        let (parts, body) = response.into_parts();
        let mut response = http::Response::builder()
            .status(parts.status)
            .version(parts.version)
            .url(request.url().clone())
            .body(reqwest::Body::wrap_stream(body.into_data_stream().map(move |chunk| {
                let _permit = &permit;
                chunk
            })))
            .map_err(io::Error::other)?;
        *response.headers_mut() = parts.headers;
        Ok(reqwest::Response::from(response))
    }
}

#[cfg(test)]
mod tests;
