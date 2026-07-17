use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{
    ws::{Message, WebSocket},
    Path, State, WebSocketUpgrade,
};
use axum::response::IntoResponse;
use futures::{SinkExt, StreamExt};
use tokio::net::UnixStream;
use tokio_tungstenite::{client_async, tungstenite::protocol::Message as TungsteniteMessage};

use crate::{terminal, AppState};

const GUI_WS_MAX_MESSAGE_BYTES: usize = 64 * 1024 * 1024;

pub async fn handle_gui_ws(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    if let Err(message) = terminal::validate_vm_id(&id) {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({"error": message})),
        )
            .into_response();
    }

    let uds_path = terminal::terminal_uds_path(&state.uds_path, &id);
    ws.max_message_size(GUI_WS_MAX_MESSAGE_BYTES)
        // xpra-html5 requests this subprotocol. The gateway unwraps the
        // WebSocket framing before the process writes bytes to AF_VSOCK.
        .protocols(["binary"])
        .on_upgrade(move |socket| handle_socket(socket, uds_path))
        .into_response()
}

async fn handle_socket(mut browser: WebSocket, uds_path: PathBuf) {
    let stream = match UnixStream::connect(&uds_path).await {
        Ok(stream) => stream,
        Err(error) => {
            tracing::error!(path = %uds_path.display(), %error, "GUI process relay unavailable");
            close_unavailable(&mut browser, "VM GUI not available").await;
            return;
        }
    };

    let (process, _) = match client_async("ws://localhost/gui", stream).await {
        Ok(connection) => connection,
        Err(error) => {
            tracing::error!(%error, "GUI process WebSocket handshake failed");
            close_unavailable(&mut browser, "VM GUI handshake failed").await;
            return;
        }
    };

    tracing::info!(path = %uds_path.display(), "GUI browser relay connected");
    let (mut browser_write, mut browser_read) = browser.split();
    let (mut process_write, mut process_read) = process.split();

    let browser_to_process = async {
        while let Some(message) = browser_read.next().await {
            let outgoing = match message {
                Ok(Message::Binary(bytes)) => TungsteniteMessage::Binary(bytes),
                Ok(Message::Ping(bytes)) => TungsteniteMessage::Ping(bytes),
                Ok(Message::Pong(bytes)) => TungsteniteMessage::Pong(bytes),
                Ok(Message::Close(frame)) => {
                    let frame =
                        frame.map(
                            |frame| tokio_tungstenite::tungstenite::protocol::CloseFrame {
                                code: frame.code.into(),
                                reason: frame.reason.to_string().into(),
                            },
                        );
                    let _ = process_write.send(TungsteniteMessage::Close(frame)).await;
                    break;
                }
                // Xpra is binary; never reinterpret or forward browser text.
                Ok(Message::Text(_)) => continue,
                Err(_) => break,
            };
            if process_write.send(outgoing).await.is_err() {
                break;
            }
        }
    };

    let process_to_browser = async {
        while let Some(message) = process_read.next().await {
            let outgoing = match message {
                Ok(TungsteniteMessage::Binary(bytes)) => Message::Binary(bytes),
                Ok(TungsteniteMessage::Ping(bytes)) => Message::Ping(bytes),
                Ok(TungsteniteMessage::Pong(bytes)) => Message::Pong(bytes),
                Ok(TungsteniteMessage::Close(frame)) => {
                    let frame = frame.map(|frame| axum::extract::ws::CloseFrame {
                        code: frame.code.into(),
                        reason: frame.reason.to_string().into(),
                    });
                    let _ = browser_write.send(Message::Close(frame)).await;
                    break;
                }
                Ok(TungsteniteMessage::Text(_)) | Ok(TungsteniteMessage::Frame(_)) => continue,
                Err(_) => break,
            };
            if browser_write.send(outgoing).await.is_err() {
                break;
            }
        }
    };

    tokio::pin!(browser_to_process);
    tokio::pin!(process_to_browser);
    tokio::select! {
        _ = &mut browser_to_process => {}
        _ = &mut process_to_browser => {}
    }
    tracing::info!(path = %uds_path.display(), "GUI browser relay disconnected");
}

async fn close_unavailable(socket: &mut WebSocket, reason: &'static str) {
    let _ = socket
        .send(Message::Close(Some(axum::extract::ws::CloseFrame {
            code: 1011,
            reason: reason.into(),
        })))
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_and_process_share_a_bounded_frame_ceiling() {
        let rgba_4k_frame = 3840 * 2160 * 4;
        assert!(GUI_WS_MAX_MESSAGE_BYTES >= rgba_4k_frame);
        assert_eq!(GUI_WS_MAX_MESSAGE_BYTES, 64 * 1024 * 1024);
    }
}
