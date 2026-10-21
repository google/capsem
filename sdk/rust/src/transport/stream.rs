//! Authenticated `capsem.stream.v2`; dropping the socket cancels the stream.

use super::{CallOptions, Transport};
use crate::{Error, Result};
use capsem_api::stream::{self, ServerFrame, StreamChannel, StreamControl, StreamStatus};
use futures::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, http::HeaderValue, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

#[derive(Debug, PartialEq, Eq)]
pub enum StreamEvent {
    Output(Vec<u8>),
    ErrorOutput(Vec<u8>),
    Terminal {
        state: Box<capsem_api::stream::TerminalState>,
    },
    Exit {
        code: i32,
        truncated: bool,
    },
}

pub struct GatewayStream {
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
}

impl Transport {
    /// Authenticate and wait for the start receipt within one request deadline.
    /// Established terminals may remain idle; callers cancel by dropping them.
    pub async fn open_stream(&self, id: &str, start: StreamControl, options: CallOptions) -> Result<GatewayStream> {
        let timeout = options.timeout.unwrap_or(self.timeout);
        if timeout.is_zero() {
            return Err(Error::InvalidInput("request timeout must be positive"));
        }
        let mut url = self.operation_url("/vms/{id}/stream", &[("id", id)])?;
        let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
        url.set_scheme(scheme).expect("HTTP URL converts to WebSocket");
        let mut request = url.as_str().into_client_request().map_err(stream_error)?;
        request
            .headers_mut()
            .insert("authorization", self.authorization.clone());
        request.headers_mut().insert(
            "sec-websocket-protocol",
            HeaderValue::from_static(stream::STREAM_SUBPROTOCOL),
        );
        tokio::time::timeout(timeout, async {
            let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
                .max_message_size(Some(stream::MAX_STREAM_FRAME_BYTES))
                .max_frame_size(Some(stream::MAX_STREAM_FRAME_BYTES));
            let (socket, response) = tokio_tungstenite::connect_async_with_config(request, Some(config), false)
                .await
                .map_err(stream_error)?;
            if response
                .headers()
                .get("sec-websocket-protocol")
                .map(HeaderValue::as_bytes)
                != Some(stream::STREAM_SUBPROTOCOL.as_bytes())
            {
                return Err(Error::InvalidResponse("gateway did not negotiate capsem.stream.v2"));
            }
            let mut attached = GatewayStream { socket };
            attached.send_frame(stream::encode_control(&start)).await?;
            match attached.next_frame().await? {
                Frame::Status(StreamStatus::Started) => {
                    if matches!(
                        start,
                        StreamControl::Start {
                            kind: stream::StreamKind::Terminal,
                            ..
                        }
                    ) {
                        attached
                            .send_frame(stream::encode_control(&StreamControl::Focus {
                                cols: capsem_api::stream::DEFAULT_TERMINAL_COLS as u16,
                                rows: capsem_api::stream::DEFAULT_TERMINAL_ROWS as u16,
                            }))
                            .await?;
                    }
                    Ok(attached)
                }
                Frame::Status(StreamStatus::Error { message }) => Err(Error::Stream(message)),
                _ => Err(Error::InvalidResponse("gateway stream did not start")),
            }
        })
        .await
        .map_err(|_| Error::Stream("gateway stream startup timed out".into()))?
    }
}

fn stream_error(error: tokio_tungstenite::tungstenite::Error) -> Error {
    match error {
        tokio_tungstenite::tungstenite::Error::Http(response) => Error::Http {
            status: response.status().as_u16(),
            body: response.into_body().unwrap_or_default(),
        },
        error => Error::Stream(error.to_string()),
    }
}

enum Frame {
    Output(Vec<u8>),
    ErrorOutput(Vec<u8>),
    Status(StreamStatus),
}

impl GatewayStream {
    async fn send_frame(&mut self, frame: Vec<u8>) -> Result<()> {
        self.socket
            .send(Message::Binary(frame.into()))
            .await
            .map_err(stream_error)
    }

    pub async fn send_stdin(&mut self, bytes: &[u8]) -> Result<()> {
        self.send_frame(stream::encode_data(StreamChannel::Stdin, bytes)).await
    }

    /// Send a typed control, including focus or resize, on this stream.
    pub async fn control(&mut self, control: &StreamControl) -> Result<()> {
        self.send_frame(stream::encode_control(control)).await
    }

    pub async fn close_stdin(&mut self) -> Result<()> {
        self.send_frame(stream::encode_control(&StreamControl::CloseStdin))
            .await
    }

    async fn next_frame(&mut self) -> Result<Frame> {
        loop {
            match self.socket.next().await {
                Some(Ok(Message::Binary(bytes))) => {
                    return match stream::decode_server_frame(&bytes)
                        .map_err(|error| Error::Stream(error.to_string()))?
                    {
                        ServerFrame::Stdout(data) => Ok(Frame::Output(data.to_vec())),
                        ServerFrame::Stderr(data) => Ok(Frame::ErrorOutput(data.to_vec())),
                        ServerFrame::Status(status) => Ok(Frame::Status(status)),
                    };
                }
                Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => continue,
                Some(Ok(Message::Text(_))) => return Err(Error::InvalidResponse("gateway stream sent a text frame")),
                Some(Ok(Message::Close(_))) | None => return Err(Error::Stream("gateway stream closed".into())),
                Some(Err(error)) => return Err(stream_error(error)),
            }
        }
    }

    /// The next display/exit event for byte-oriented consumers. Structured
    /// clients use `next` to receive terminal metadata as well.
    pub async fn next_output(&mut self) -> Result<StreamEvent> {
        loop {
            let event = self.next().await?;
            if !matches!(event, StreamEvent::Terminal { .. }) {
                return Ok(event);
            }
        }
    }

    pub async fn next(&mut self) -> Result<StreamEvent> {
        loop {
            match self.next_frame().await? {
                Frame::Output(bytes) => return Ok(StreamEvent::Output(bytes)),
                Frame::ErrorOutput(bytes) => return Ok(StreamEvent::ErrorOutput(bytes)),
                Frame::Status(StreamStatus::Exit { code, truncated }) => {
                    return Ok(StreamEvent::Exit { code, truncated })
                }
                Frame::Status(StreamStatus::Error { message }) => return Err(Error::Stream(message)),
                Frame::Status(StreamStatus::Started) => continue,
                Frame::Status(StreamStatus::Terminal { state }) => return Ok(StreamEvent::Terminal { state }),
            }
        }
    }
}

#[cfg(test)]
mod tests;
