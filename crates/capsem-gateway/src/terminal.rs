use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{
    ws::{Message, WebSocket},
    Path, State, WebSocketUpgrade,
};
use axum::response::IntoResponse;
use futures::{sink::SinkExt, stream::StreamExt, Sink};
use tokio::net::UnixStream;
use tokio::time::{timeout, Duration};
use tokio_tungstenite::{client_async, tungstenite::protocol::Message as TungsteniteMessage};

use crate::AppState;

const TERMINAL_RELAY_BATCH_MAX_BYTES: usize = 64 * 1024;
const TERMINAL_RELAY_BATCH_FLUSH: Duration = Duration::from_millis(16);

enum TerminalRelayBatch {
    Text(String),
    Binary(Vec<u8>),
}

fn queue_text_batch(
    pending: &mut Option<TerminalRelayBatch>,
    text: String,
) -> Option<TerminalRelayBatch> {
    if text.is_empty() {
        return None;
    }
    match pending {
        Some(TerminalRelayBatch::Text(buffer))
            if buffer.len() + text.len() <= TERMINAL_RELAY_BATCH_MAX_BYTES =>
        {
            buffer.push_str(&text);
            if buffer.len() >= TERMINAL_RELAY_BATCH_MAX_BYTES {
                pending.take()
            } else {
                None
            }
        }
        Some(TerminalRelayBatch::Text(_)) | Some(TerminalRelayBatch::Binary(_)) => {
            let flush = pending.take();
            *pending = Some(TerminalRelayBatch::Text(text));
            flush
        }
        None => {
            *pending = Some(TerminalRelayBatch::Text(text));
            None
        }
    }
}

fn queue_binary_batch(
    pending: &mut Option<TerminalRelayBatch>,
    bytes: Vec<u8>,
) -> Option<TerminalRelayBatch> {
    if bytes.is_empty() {
        return None;
    }
    match pending {
        Some(TerminalRelayBatch::Binary(buffer))
            if buffer.len() + bytes.len() <= TERMINAL_RELAY_BATCH_MAX_BYTES =>
        {
            buffer.extend_from_slice(&bytes);
            if buffer.len() >= TERMINAL_RELAY_BATCH_MAX_BYTES {
                pending.take()
            } else {
                None
            }
        }
        Some(TerminalRelayBatch::Text(_)) | Some(TerminalRelayBatch::Binary(_)) => {
            let flush = pending.take();
            *pending = Some(TerminalRelayBatch::Binary(bytes));
            flush
        }
        None => {
            *pending = Some(TerminalRelayBatch::Binary(bytes));
            None
        }
    }
}

async fn send_batch_to_client<W>(writer: &mut W, batch: TerminalRelayBatch) -> bool
where
    W: Sink<Message> + Unpin,
{
    match batch {
        TerminalRelayBatch::Text(text) => writer.send(Message::Text(text.into())).await.is_ok(),
        TerminalRelayBatch::Binary(bytes) => {
            writer.send(Message::Binary(bytes.into())).await.is_ok()
        }
    }
}

async fn flush_batch_to_client<W>(writer: &mut W, pending: &mut Option<TerminalRelayBatch>) -> bool
where
    W: Sink<Message> + Unpin,
{
    match pending.take() {
        Some(batch) => send_batch_to_client(writer, batch).await,
        None => true,
    }
}

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
        loop {
            let msg = client_read.next().await;
            match msg {
                Some(Ok(Message::Text(t))) => {
                    if process_write
                        .send(TungsteniteMessage::Text(t.to_string().into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Some(Ok(Message::Binary(b))) => {
                    if process_write
                        .send(TungsteniteMessage::Binary(b.to_vec().into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Some(Ok(Message::Ping(p))) => {
                    let vec = p.to_vec();
                    if process_write
                        .send(TungsteniteMessage::Ping(vec.into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Some(Ok(Message::Pong(p))) => {
                    let vec = p.to_vec();
                    if process_write
                        .send(TungsteniteMessage::Pong(vec.into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Some(Ok(Message::Close(c))) => {
                    let frame = c.map(|f| tokio_tungstenite::tungstenite::protocol::CloseFrame {
                        code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::from(f.code),
                        reason: f.reason.to_string().into(),
                    });
                    let _ = process_write.send(TungsteniteMessage::Close(frame)).await;
                    break;
                }
                Some(Err(_)) => {
                    break;
                }
                None => {
                    break;
                }
            }
        }
    });

    let mut p2c = tokio::spawn(async move {
        let mut pending: Option<TerminalRelayBatch> = None;
        loop {
            let msg = if pending.is_some() {
                match timeout(TERMINAL_RELAY_BATCH_FLUSH, process_read.next()).await {
                    Ok(msg) => msg,
                    Err(_) => {
                        if !flush_batch_to_client(&mut client_write, &mut pending).await {
                            break;
                        }
                        continue;
                    }
                }
            } else {
                process_read.next().await
            };
            match msg {
                Some(Ok(TungsteniteMessage::Text(t))) => {
                    let s: String = t.to_string();
                    if let Some(batch) = queue_text_batch(&mut pending, s) {
                        if !send_batch_to_client(&mut client_write, batch).await {
                            break;
                        }
                    }
                }
                Some(Ok(TungsteniteMessage::Binary(b))) => {
                    if let Some(batch) = queue_binary_batch(&mut pending, b.to_vec()) {
                        if !send_batch_to_client(&mut client_write, batch).await {
                            break;
                        }
                    }
                }
                Some(Ok(TungsteniteMessage::Ping(p))) => {
                    if !flush_batch_to_client(&mut client_write, &mut pending).await {
                        break;
                    }
                    let vec = p.to_vec();
                    if client_write.send(Message::Ping(vec.into())).await.is_err() {
                        break;
                    }
                }
                Some(Ok(TungsteniteMessage::Pong(p))) => {
                    if !flush_batch_to_client(&mut client_write, &mut pending).await {
                        break;
                    }
                    let vec = p.to_vec();
                    if client_write.send(Message::Pong(vec.into())).await.is_err() {
                        break;
                    }
                }
                Some(Ok(TungsteniteMessage::Close(c))) => {
                    if !flush_batch_to_client(&mut client_write, &mut pending).await {
                        break;
                    }
                    let frame = c.map(|f| axum::extract::ws::CloseFrame {
                        code: f.code.into(),
                        reason: f.reason.to_string().into(),
                    });
                    let _ = client_write.send(Message::Close(frame)).await;
                    break;
                }
                Some(Ok(TungsteniteMessage::Frame(_))) => {}
                Some(Err(_)) => {
                    let _ = flush_batch_to_client(&mut client_write, &mut pending).await;
                    break;
                }
                None => {
                    let _ = flush_batch_to_client(&mut client_write, &mut pending).await;
                    break;
                }
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
