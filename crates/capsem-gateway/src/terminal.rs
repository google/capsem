use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{
    ws::{Message, WebSocket},
    Path, State, WebSocketUpgrade,
};
use axum::response::IntoResponse;
use futures::{sink::SinkExt, stream::StreamExt};
use tokio::net::UnixStream;
use tokio_tungstenite::{client_async, tungstenite::protocol::Message as TungsteniteMessage};

use crate::AppState;

/// Validate VM ID: alphanumeric, hyphens, underscores. Must start with
/// alphanumeric, length 1-64. Matches capsem-service's `validate_vm_name`.
fn validate_vm_id(id: &str) -> Result<(), &'static str> {
    if id.is_empty() {
        return Err("VM id cannot be empty");
    }
    if id.len() > 64 {
        return Err("VM id too long (max 64 characters)");
    }
    if !id.chars().next().unwrap().is_ascii_alphanumeric() {
        return Err("VM id must start with a letter or digit");
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("VM id must contain only letters, digits, hyphens, and underscores");
    }
    Ok(())
}

/// Derive the per-VM WebSocket UDS path from the service socket and VM ID.
fn terminal_uds_path(service_uds: &std::path::Path, id: &str) -> PathBuf {
    let run_dir = service_uds
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("/tmp"));
    run_dir.join("instances").join(format!("{}-ws.sock", id))
}

pub async fn handle_terminal_ws(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    if let Err(msg) = validate_vm_id(&id) {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({"error": msg})),
        )
            .into_response();
    }

    let uds_path = terminal_uds_path(&state.uds_path, &id);

    ws.on_upgrade(move |socket| handle_socket(socket, uds_path))
        .into_response()
}

async fn handle_socket(mut client_ws: WebSocket, uds_path: PathBuf) {
    let stream = match UnixStream::connect(&uds_path).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(
                "Failed to connect to process WS UDS {}: {}",
                uds_path.display(),
                e
            );
            let _ = client_ws
                .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                    code: 1011, // unexpected condition
                    reason: "VM not available".into(),
                })))
                .await;
            return;
        }
    };

    let req = "ws://localhost/terminal".to_string();
    let (process_ws, _) = match client_async(&req, stream).await {
        Ok(res) => res,
        Err(e) => {
            tracing::error!("WebSocket handshake with process failed: {}", e);
            let _ = client_ws
                .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                    code: 1011,
                    reason: "VM handshake failed".into(),
                })))
                .await;
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
                    if process_write
                        .send(TungsteniteMessage::Text(s.into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(Message::Binary(b)) => {
                    let vec = b.to_vec();
                    if process_write
                        .send(TungsteniteMessage::Binary(vec.into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(Message::Ping(p)) => {
                    let vec = p.to_vec();
                    if process_write
                        .send(TungsteniteMessage::Ping(vec.into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(Message::Pong(p)) => {
                    let vec = p.to_vec();
                    if process_write
                        .send(TungsteniteMessage::Pong(vec.into()))
                        .await
                        .is_err()
                    {
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
                    if client_write
                        .send(Message::Binary(vec.into()))
                        .await
                        .is_err()
                    {
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

#[cfg(test)]
mod tests;
