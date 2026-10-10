//! Responses message transport after the ordinary HTTP upgrade admission.
use super::*;
use futures::{Sink, SinkExt, StreamExt};
use hooks::ConnMeta;
use tokio::sync::mpsc;
use tokio_tungstenite::{
    tungstenite::{
        protocol::{Role, WebSocketConfig},
        Message,
    },
    WebSocketStream,
};
mod turns;

pub(super) struct Context {
    pub pipeline: Arc<pipeline::Pipeline>,
    pub engine: Arc<crate::net::proxy_engine::ProxyEngine>,
    pub conn: ConnMeta,
    pub ip: Option<IpAddr>,
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    pub request_headers: String,
    pub response_headers: String,
    pub credential_ref: Option<String>,
    pub credential_observations: Vec<crate::credential_broker::CredentialObservation>,
}

pub(super) async fn bridge<C, U>(client: C, upstream: U, context: Context) -> anyhow::Result<()>
where
    C: AsyncRead + AsyncWrite + Unpin,
    U: AsyncRead + AsyncWrite + Unpin,
{
    let limits = WebSocketConfig::default()
        .max_message_size(Some(AI_BODY_CAPTURE_LIMIT))
        .max_frame_size(Some(AI_BODY_CAPTURE_LIMIT));
    let client = WebSocketStream::from_raw_socket(client, Role::Server, Some(limits)).await;
    let upstream = WebSocketStream::from_raw_socket(upstream, Role::Client, Some(limits)).await;
    let (guest_writer, mut guest_reader) = client.split();
    let (upstream_writer, mut upstream_reader) = upstream.split();
    let (to_guest, guest_queue) = mpsc::channel(4);
    let (to_upstream, upstream_queue) = mpsc::channel(4);
    let mut turns = turns::Turns::default();
    // Writers run independently so neither peer's send buffer prevents reads.
    // These scoped futures own both halves; there are no detached writer tasks.
    let driver = async {
        loop {
            let (guest, message) = tokio::select! {
                message = guest_reader.next() => (true, message),
                message = upstream_reader.next() => (false, message),
            };
            let Some(message) = message else {
                break;
            };
            let message = match message {
                Ok(message) => message,
                Err(tokio_tungstenite::tungstenite::Error::ConnectionClosed) => break,
                Err(error) => return Err(error.into()),
            };
            if let Message::Text(text) = message {
                if guest {
                    match turns.request(text.as_str(), &context).await? {
                        turns::Request::Forward(text) => to_upstream.send(Message::Text(text.into())).await?,
                        turns::Request::Refuse(text) => to_guest.send(Message::Text(text.into())).await?,
                    }
                } else {
                    for text in turns.response(text.as_str(), &context).await? {
                        to_guest.send(Message::Text(text.into())).await?;
                    }
                }
            } else if let Message::Close(reason) = message {
                // Acknowledge this peer and close the other; neither must wait
                // indefinitely for a second read to flush its close response.
                to_guest.send(Message::Close(reason.clone())).await?;
                to_upstream.send(Message::Close(reason)).await?;
                break;
            } else if matches!(message, Message::Binary(_)) {
                anyhow::bail!("Responses WebSocket messages must be JSON text");
            } else {
                let destination = if guest { &to_upstream } else { &to_guest };
                destination.send(message).await?;
            }
        }
        drop(to_guest);
        drop(to_upstream);
        Ok::<(), anyhow::Error>(())
    };
    let result = tokio::try_join!(
        write(guest_writer, guest_queue),
        write(upstream_writer, upstream_queue),
        driver
    );
    // Even an EOF, malformed frame or writer failure finalizes the captured
    // partial turns through the DB-owned completion rail.
    turns.finish(&context).await;
    result.map(|_| ())
}

async fn write<S>(mut sink: S, mut queue: mpsc::Receiver<Message>) -> anyhow::Result<()>
where
    S: Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    while let Some(message) = queue.recv().await {
        let control = matches!(message, Message::Close(_) | Message::Ping(_) | Message::Pong(_));
        match sink.send(message).await {
            Ok(()) => {}
            Err(tokio_tungstenite::tungstenite::Error::Protocol(
                tokio_tungstenite::tungstenite::error::ProtocolError::SendAfterClosing,
            )) if control => {
                // Reading Close already queued this socket's acknowledgement.
                // Flush that acknowledgement instead of writing a second Close.
                match sink.flush().await {
                    Ok(()) | Err(tokio_tungstenite::tungstenite::Error::ConnectionClosed) => break,
                    Err(error) => return Err(error.into()),
                }
            }
            Err(
                tokio_tungstenite::tungstenite::Error::ConnectionClosed
                | tokio_tungstenite::tungstenite::Error::AlreadyClosed,
            ) => break,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
