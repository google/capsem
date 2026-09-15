//! `capsem.stream.v1` over the service socket: the one way the CLI attaches
//! to a VM terminal, a streaming command or a container workload.

use super::*;
use capsem_api::stream::{self, ServerFrame, StreamChannel, StreamControl, StreamStatus};
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, http::HeaderValue, Message};

/// One thing a stream delivered.
#[derive(Debug, PartialEq, Eq)]
pub enum StreamEvent {
    Output(Vec<u8>),
    Exit { code: i32, truncated: bool },
}

/// An attached `/vms/{id}/stream`.
pub struct ServiceStream {
    socket: tokio_tungstenite::WebSocketStream<UnixStream>,
}

impl UdsClient {
    /// Attach to VM `vm_id` with `start`, returning once the service reports
    /// the stream started.
    pub async fn open_stream(&self, vm_id: &str, start: StreamControl) -> Result<ServiceStream> {
        let socket = match self.connect_with_timeout(ConnectMode::FailFast).await {
            Ok(socket) => socket,
            Err(error) if !self.auto_launch => {
                anyhow::bail!("cannot connect to service at {}: {error}", self.uds_path.display())
            }
            Err(_) => self.try_ensure_service().await?,
        };
        let mut request = format!("ws://localhost/vms/{}/stream", urlencoding::encode(vm_id)).into_client_request()?;
        request.headers_mut().insert(
            "sec-websocket-protocol",
            HeaderValue::from_static(stream::STREAM_SUBPROTOCOL),
        );
        let (socket, _) = tokio_tungstenite::client_async(request, socket)
            .await
            .context("open VM stream")?;
        let mut attached = ServiceStream { socket };
        attached.send_frame(stream::encode_control(&start)).await?;
        match attached.next_frame().await? {
            Frame::Status(StreamStatus::Started) => Ok(attached),
            Frame::Status(StreamStatus::Error { message }) => Err(anyhow::anyhow!(message)),
            other => Err(anyhow::anyhow!("VM stream did not start: {other:?}")),
        }
    }
}

#[derive(Debug)]
enum Frame {
    Output(Vec<u8>),
    Status(StreamStatus),
}

impl ServiceStream {
    async fn send_frame(&mut self, frame: Vec<u8>) -> Result<()> {
        self.socket
            .send(Message::Binary(frame.into()))
            .await
            .context("VM stream closed")
    }

    pub async fn send_stdin(&mut self, data: &[u8]) -> Result<()> {
        self.send_frame(stream::encode_data(StreamChannel::Stdin, data)).await
    }

    async fn next_frame(&mut self) -> Result<Frame> {
        loop {
            match self.socket.next().await {
                Some(Ok(Message::Binary(bytes))) => {
                    return match stream::decode_server_frame(&bytes).map_err(|e| anyhow::anyhow!("VM stream: {e}"))? {
                        ServerFrame::Stdout(data) | ServerFrame::Stderr(data) => Ok(Frame::Output(data.to_vec())),
                        ServerFrame::Status(status) => Ok(Frame::Status(status)),
                    };
                }
                Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => continue,
                Some(Ok(Message::Text(_))) => anyhow::bail!("VM stream sent a text frame"),
                Some(Ok(Message::Close(_))) | None => anyhow::bail!("VM stream closed"),
                Some(Err(error)) => return Err(error).context("VM stream"),
            }
        }
    }

    /// The next output or the exit. An error status or a stream that closes
    /// without an exit is an error carrying the service's reason.
    pub async fn next(&mut self) -> Result<StreamEvent> {
        loop {
            match self.next_frame().await? {
                Frame::Output(data) => return Ok(StreamEvent::Output(data)),
                Frame::Status(StreamStatus::Exit { code, truncated }) => {
                    return Ok(StreamEvent::Exit { code, truncated })
                }
                Frame::Status(StreamStatus::Error { message }) => anyhow::bail!(message),
                Frame::Status(StreamStatus::Started) => continue,
            }
        }
    }
}

#[cfg(test)]
mod tests;
