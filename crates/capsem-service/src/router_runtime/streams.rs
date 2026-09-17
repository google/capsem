//! `GET /vms/{id}/stream`: one `capsem.stream.v1` WebSocket per terminal,
//! streaming exec or attached container, relayed to a stream-role VM-owner
//! connection. Frames are translated, never buffered past one message, so the
//! client's reading speed is the guest's writing speed.

use super::*;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use capsem_api::stream::{self, ClientFrame, StreamChannel, StreamControl, StreamKind, StreamStatus};
use futures::{SinkExt, StreamExt};

/// How long a client may take to send its `start` control message.
const START_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// How often an exec waiting for stdin credit pings the client it is not
/// reading, so a client that left is noticed and its command cancelled.
const CREDIT_WAIT_PING: std::time::Duration = std::time::Duration::from_secs(1);

type OwnerChannel = (
    capsem_foundation::ipc_channel::Sender<ServiceToProcess>,
    capsem_foundation::ipc_channel::Receiver<ProcessToService>,
);

pub(crate) async fn handle_stream(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    upgrade: WebSocketUpgrade,
) -> Result<axum::response::Response, AppError> {
    let upgrade = upgrade.protocols([stream::STREAM_SUBPROTOCOL]);
    if upgrade.selected_protocol().is_none() {
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            format!(
                "stream requires the {} WebSocket subprotocol",
                stream::STREAM_SUBPROTOCOL
            ),
        ));
    }
    let uds_path = running_uds_path(&state, &id)?;
    Ok(upgrade
        .max_message_size(stream::MAX_STREAM_FRAME_BYTES)
        .on_upgrade(move |socket| session(state, id, uds_path, socket)))
}

async fn session(state: Arc<ServiceState>, id: String, uds_path: PathBuf, socket: WebSocket) {
    let (mut client_tx, mut client_rx) = socket.split();
    let outcome = async {
        let (kind, command) = tokio::time::timeout(START_TIMEOUT, first_start(&mut client_rx))
            .await
            .map_err(|_| "no start control message".to_string())??;
        // Claim a staged container before touching its owner: a refused attach
        // must not start anything.
        let claim = match kind {
            StreamKind::Container => Some(state.containers.claim_attach(&id)?),
            StreamKind::Terminal | StreamKind::Exec => None,
        };
        let owner = async {
            wait_for_vm_ready(&uds_path, 30, Some(&state), Some(&id)).await?;
            open_owner(&uds_path).await
        }
        .await;
        match (kind, claim) {
            (StreamKind::Terminal, _) => terminal(owner?, &mut client_tx, &mut client_rx).await,
            (StreamKind::Exec, _) => {
                let command = command.expect("decoder requires an exec command");
                exec(&state, owner?, command, &mut client_tx, &mut client_rx)
                    .await
                    .map(drop)
            }
            (StreamKind::Container, claim) => {
                let Some(generation) = claim else {
                    return Err("container stream was not claimed".to_string());
                };
                let result = match owner {
                    Ok(owner) => {
                        if let Err(error) = container_setup::record_launched(&state, &id) {
                            warn!(vm_id = id.as_str(), %error, "attached container launch record not written");
                        }
                        let command = capsem_core::container::LAUNCH_COMMAND.to_string();
                        exec(&state, owner, command, &mut client_tx, &mut client_rx).await
                    }
                    Err(error) => Err(error),
                };
                let ended = result
                    .clone()
                    .and_then(|exit| exit.ok_or_else(|| "attached client left".to_string()));
                state.containers.finish_attach(&id, generation, ended);
                result.map(drop)
            }
        }
    }
    .await;
    if let Err(message) = outcome {
        let _ = client_tx
            .send(Message::Binary(
                stream::encode_status(&StreamStatus::Error { message }).into(),
            ))
            .await;
    }
    let _ = client_tx
        .send(Message::Close(Some(CloseFrame {
            code: axum::extract::ws::close_code::NORMAL,
            reason: "".into(),
        })))
        .await;
}

async fn first_start(
    client_rx: &mut futures::stream::SplitStream<WebSocket>,
) -> Result<(StreamKind, Option<String>), String> {
    match next_client_frame(client_rx).await? {
        Some(ClientControl::Control(StreamControl::Start { kind, command })) => Ok((kind, command)),
        Some(_) => Err("the first stream message must be start".into()),
        None => Err("client left before starting".into()),
    }
}

enum ClientControl {
    Stdin(Vec<u8>),
    Control(StreamControl),
}

/// The next meaningful client frame, `None` when the client left.
async fn next_client_frame(
    client_rx: &mut futures::stream::SplitStream<WebSocket>,
) -> Result<Option<ClientControl>, String> {
    loop {
        match client_rx.next().await {
            None | Some(Ok(Message::Close(_))) => return Ok(None),
            Some(Ok(Message::Binary(bytes))) => {
                return match stream::decode_client_frame(&bytes).map_err(|e| e.to_string())? {
                    ClientFrame::Stdin(data) => Ok(Some(ClientControl::Stdin(data.to_vec()))),
                    ClientFrame::Control(control) => Ok(Some(ClientControl::Control(control))),
                };
            }
            Some(Ok(Message::Text(_))) => return Err("stream frames are binary".into()),
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
            Some(Err(error)) => return Err(format!("stream transport: {error}")),
        }
    }
}

async fn open_owner(uds_path: &StdPath) -> Result<OwnerChannel, String> {
    let socket = tokio::net::UnixStream::connect(uds_path)
        .await
        .and_then(|socket| socket.into_std())
        .map_err(|e| format!("VM owner unavailable: {e}"))?;
    let (socket, _) = capsem_foundation::ipc_handshake::negotiate_initiator_off_worker(
        socket,
        capsem_proto::handshake::STREAM_PEER_ID,
        capsem_foundation::telemetry::current_parent_traceparent(),
    )
    .await
    .map_err(|e| format!("VM owner handshake: {e}"))?;
    capsem_foundation::ipc_channel::channel_from_std(socket).map_err(|e| format!("VM owner channel: {e}"))
}

async fn send_data(
    client_tx: &mut futures::stream::SplitSink<WebSocket, Message>,
    channel: StreamChannel,
    data: &[u8],
) -> Result<(), String> {
    client_tx
        .send(Message::Binary(stream::encode_data(channel, data).into()))
        .await
        .map_err(|e| format!("client left: {e}"))
}

async fn send_status(
    client_tx: &mut futures::stream::SplitSink<WebSocket, Message>,
    status: &StreamStatus,
) -> Result<(), String> {
    client_tx
        .send(Message::Binary(stream::encode_status(status).into()))
        .await
        .map_err(|e| format!("client left: {e}"))
}

async fn terminal(
    (owner_tx, owner_rx): OwnerChannel,
    client_tx: &mut futures::stream::SplitSink<WebSocket, Message>,
    client_rx: &mut futures::stream::SplitStream<WebSocket>,
) -> Result<(), String> {
    let owner_closed = |e: std::io::Error| format!("VM owner closed: {e}");
    owner_tx
        .send(ServiceToProcess::StartTerminalStream)
        .await
        .map_err(owner_closed)?;
    send_status(client_tx, &StreamStatus::Started).await?;
    let result = async {
        loop {
            tokio::select! {
            frame = next_client_frame(client_rx) => match frame? {
                None => break Ok(()),
                Some(ClientControl::Stdin(data)) => {
                    owner_tx.send(ServiceToProcess::TerminalInput { data }).await.map_err(owner_closed)?;
                }
                Some(ClientControl::Control(StreamControl::Resize { cols, rows })) => {
                    owner_tx.send(ServiceToProcess::TerminalResize { cols, rows }).await.map_err(owner_closed)?;
                }
                Some(ClientControl::Control(StreamControl::CloseStdin)) => {}
                Some(ClientControl::Control(StreamControl::Start { .. })) => break Err("stream already started".into()),
            },
            message = owner_rx.recv() => match message.map_err(owner_closed)? {
                ProcessToService::TerminalOutput { data } => send_data(client_tx, StreamChannel::Stdout, &data).await?,
                ProcessToService::TerminalStreamEnded { reason } => break Err(reason),
                _ => {}
            },
            }
        }
    }
    .await;
    // Late output must not follow the client's decision to leave.
    let _ = owner_tx.send(ServiceToProcess::StopTerminalStream).await;
    result
}

/// Run `command` attached. `Ok(Some(code))` is its exit; `Ok(None)` means the
/// client left first and the command was cancelled.
async fn exec(
    state: &ServiceState,
    (owner_tx, owner_rx): OwnerChannel,
    command: String,
    client_tx: &mut futures::stream::SplitSink<WebSocket, Message>,
    client_rx: &mut futures::stream::SplitStream<WebSocket>,
) -> Result<Option<i32>, String> {
    let owner_closed = |e: std::io::Error| format!("VM owner closed: {e}");
    let job = state.next_job_id();
    owner_tx
        .send(ServiceToProcess::ExecStream { id: job, command })
        .await
        .map_err(owner_closed)?;
    send_status(client_tx, &StreamStatus::Started).await?;
    let mut stdin_closed = false;
    // Stdin frames the owner can still queue. Client frames are read only
    // while there is credit, so the owner's read loop never waits on a full
    // stdin queue and stays free to process CancelExec.
    let mut credit = capsem_proto::EXEC_STDIN_WINDOW;
    let mut ping = tokio::time::interval(CREDIT_WAIT_PING);
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let result = loop {
        tokio::select! {
            frame = next_client_frame(client_rx), if credit > 0 => match frame? {
                None => break Ok(None),
                Some(ClientControl::Stdin(_)) if stdin_closed => break Err("exec stdin is already closed".into()),
                Some(ClientControl::Stdin(data)) => {
                    credit -= 1;
                    owner_tx.send(ServiceToProcess::ExecStreamInput { id: job, data }).await.map_err(owner_closed)?;
                }
                Some(ClientControl::Control(StreamControl::CloseStdin)) if !stdin_closed => {
                    stdin_closed = true;
                    credit -= 1;
                    owner_tx.send(ServiceToProcess::ExecStreamCloseStdin { id: job }).await.map_err(owner_closed)?;
                }
                Some(ClientControl::Control(StreamControl::CloseStdin)) => {}
                Some(ClientControl::Control(StreamControl::Start { .. })) => break Err("stream already started".into()),
                Some(ClientControl::Control(StreamControl::Resize { .. })) => {}
            },
            _ = ping.tick(), if credit == 0 => {
                if client_tx.send(Message::Ping(Default::default())).await.is_err() {
                    break Ok(None);
                }
            }
            message = owner_rx.recv() => match message.map_err(owner_closed)? {
                ProcessToService::ExecInputConsumed { id } if id == job => {
                    credit = (credit + 1).min(capsem_proto::EXEC_STDIN_WINDOW);
                }
                ProcessToService::ExecOutput { id, channel, data } if id == job => {
                    let channel = match channel {
                        capsem_proto::ExecOutputChannel::Stdout => StreamChannel::Stdout,
                        capsem_proto::ExecOutputChannel::Stderr => StreamChannel::Stderr,
                    };
                    send_data(client_tx, channel, &data).await?;
                }
                ProcessToService::ExecResult { id, exit_code, stderr, truncated, .. } if id == job => {
                    if exit_code < 0 && !stderr.is_empty() {
                        break Err(String::from_utf8_lossy(&stderr).into_owned());
                    }
                    send_status(client_tx, &StreamStatus::Exit { code: exit_code, truncated }).await?;
                    break Ok(Some(exit_code));
                }
                _ => {}
            },
        }
    };
    if !matches!(result, Ok(Some(_))) {
        let _ = owner_tx.send(ServiceToProcess::CancelExec { id: job }).await;
    }
    result
}

#[cfg(test)]
mod tests;
