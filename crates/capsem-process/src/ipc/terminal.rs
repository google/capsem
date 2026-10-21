//! Ordered state/render delivery from the session terminal to one viewer.
use capsem_core::terminal::TerminalUpdate;
use capsem_proto::ipc::ProcessToService;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};

async fn send(out: &mpsc::Sender<ProcessToService>, update: &TerminalUpdate) -> Result<(), ()> {
    out.send(ProcessToService::TerminalState {
        state: Box::new(update.state.clone()),
    })
    .await
    .map_err(|_| ())?;
    for data in update.render.chunks(capsem_proto::TERMINAL_RENDER_CHUNK_BYTES) {
        out.send(ProcessToService::TerminalOutput { data: data.to_vec() })
            .await
            .map_err(|_| ())?;
    }
    Ok(())
}

pub(super) fn stream(
    out: mpsc::Sender<ProcessToService>,
    snapshot: TerminalUpdate,
    mut updates: broadcast::Receiver<Arc<TerminalUpdate>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if send(&out, &snapshot).await.is_err() {
            return;
        }
        let reason = loop {
            match updates.recv().await {
                Ok(update) => {
                    if send(&out, &update).await.is_err() {
                        return;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    break "terminal view fell behind; reconnect for current state".to_string()
                }
                Err(broadcast::error::RecvError::Closed) => break "terminal closed".to_string(),
            }
        };
        let _ = out.send(ProcessToService::TerminalStreamEnded { reason }).await;
    })
}
