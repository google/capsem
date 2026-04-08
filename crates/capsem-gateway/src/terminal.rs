use std::sync::Arc;
use std::path::PathBuf;

use axum::extract::{Path, State, WebSocketUpgrade, ws::{WebSocket, Message}};
use axum::response::IntoResponse;
use futures::{sink::SinkExt, stream::StreamExt};
use tokio::net::UnixStream;
use tokio_tungstenite::{client_async, tungstenite::protocol::Message as TungsteniteMessage};

use crate::AppState;

pub async fn handle_terminal_ws(
    State(_state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    let run_dir = PathBuf::from(&home).join(".capsem/run/instances");
    let uds_path = run_dir.join(format!("{}-ws.sock", id));

    ws.on_upgrade(move |socket| handle_socket(socket, uds_path))
}

async fn handle_socket(client_ws: WebSocket, uds_path: PathBuf) {
    let stream = match UnixStream::connect(&uds_path).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to connect to process WS UDS {}: {}", uds_path.display(), e);
            return;
        }
    };

    let req = format!("ws://localhost/terminal");
    let (process_ws, _) = match client_async(&req, stream).await {
        Ok(res) => res,
        Err(e) => {
            tracing::error!("WebSocket handshake with process failed: {}", e);
            return;
        }
    };

    tracing::info!("Established WS connection to {}", uds_path.display());

    let (mut client_write, mut client_read) = client_ws.split();
    let (mut process_write, mut process_read) = process_ws.split();

    let mut c2p = tokio::spawn(async move {
        while let Some(msg) = client_read.next().await {
            match msg {
                Ok(Message::Text(t)) => {
                    let s: String = t.to_string();
                    if process_write.send(TungsteniteMessage::Text(s.into())).await.is_err() {
                        break;
                    }
                }
                Ok(Message::Binary(b)) => {
                    let vec = b.to_vec();
                    if process_write.send(TungsteniteMessage::Binary(vec.into())).await.is_err() {
                        break;
                    }
                }
                Ok(Message::Ping(p)) => {
                    let vec = p.to_vec();
                    if process_write.send(TungsteniteMessage::Ping(vec.into())).await.is_err() {
                        break;
                    }
                }
                Ok(Message::Pong(p)) => {
                    let vec = p.to_vec();
                    if process_write.send(TungsteniteMessage::Pong(vec.into())).await.is_err() {
                        break;
                    }
                }
                Ok(Message::Close(c)) => {
                    let frame = c.map(|f| tokio_tungstenite::tungstenite::protocol::CloseFrame {
                        code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::from(f.code),
                        reason: f.reason.to_string().into(),
                    });
                    let _ = process_write.send(TungsteniteMessage::Close(frame)).await;
                    break;
                }
                Err(_) => break,
            }
        }
    });

    let mut p2c = tokio::spawn(async move {
        while let Some(msg) = process_read.next().await {
            match msg {
                Ok(TungsteniteMessage::Text(t)) => {
                    let s: String = t.to_string();
                    if client_write.send(Message::Text(s.into())).await.is_err() {
                        break;
                    }
                }
                Ok(TungsteniteMessage::Binary(b)) => {
                    let vec = b.to_vec();
                    if client_write.send(Message::Binary(vec.into())).await.is_err() {
                        break;
                    }
                }
                Ok(TungsteniteMessage::Ping(p)) => {
                    let vec = p.to_vec();
                    if client_write.send(Message::Ping(vec.into())).await.is_err() {
                        break;
                    }
                }
                Ok(TungsteniteMessage::Pong(p)) => {
                    let vec = p.to_vec();
                    if client_write.send(Message::Pong(vec.into())).await.is_err() {
                        break;
                    }
                }
                Ok(TungsteniteMessage::Close(c)) => {
                    let frame = c.map(|f| axum::extract::ws::CloseFrame {
                        code: f.code.into(),
                        reason: f.reason.to_string().into(),
                    });
                    let _ = client_write.send(Message::Close(frame)).await;
                    break;
                }
                Ok(TungsteniteMessage::Frame(_)) => {}
                Err(_) => break,
            }
        }
    });

    tokio::select! {
        _ = &mut c2p => {
            tracing::info!("Client disconnected from terminal");
            p2c.abort();
        }
        _ = &mut p2c => {
            tracing::info!("Process disconnected from terminal");
            c2p.abort();
        }
    }
}
