//! One request and its correlated reply over a fresh VM-owner IPC connection.
use super::*;

#[tracing::instrument(skip_all, fields(cmd = ?std::mem::discriminant(&cmd), timeout_secs = ?timeout_secs))]
pub(crate) async fn send_ipc_command(
    uds_path: &std::path::Path,
    cmd: ServiceToProcess,
    timeout_secs: Option<u64>,
) -> Result<ProcessToService, String> {
    let stream = tokio::net::UnixStream::connect(uds_path)
        .await
        .map_err(|e| format!("failed to connect to sandbox: {e}"))?;
    let std_stream = stream
        .into_std()
        .map_err(|e| format!("failed to convert stream: {e}"))?;
    let (std_stream, _) = capsem_foundation::ipc_handshake::negotiate_initiator_off_worker(
        std_stream,
        "capsem-service",
        capsem_foundation::telemetry::current_parent_traceparent(),
    )
    .await
    .map_err(|e| format!("IPC handshake failed: {e}"))?;
    let (tx, rx): (Sender<ServiceToProcess>, Receiver<ProcessToService>) =
        channel_from_std(std_stream).map_err(|e| format!("failed to create IPC channel: {e}"))?;

    tx.send(cmd.clone())
        .await
        .map_err(|e| format!("failed to send IPC command: {e}"))?;

    let deadline = timeout_secs.map(|secs| tokio::time::Instant::now() + std::time::Duration::from_secs(secs));
    loop {
        let msg = match deadline {
            Some(deadline) => match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Ok(msg)) => msg,
                Ok(Err(e)) => {
                    error!(?e, "IPC receive error");
                    return Err(format!("IPC connection closed: {e}"));
                }
                Err(_) => {
                    let secs = timeout_secs.unwrap_or_default();
                    return Err(format!("IPC command timed out after {secs}s"));
                }
            },
            None => match rx.recv().await {
                Ok(msg) => msg,
                Err(e) => {
                    error!(?e, "IPC receive error");
                    return Err(format!("IPC connection closed: {e}"));
                }
            },
        };

        // The connection also carries lifecycle broadcasts and, for streams,
        // output chunks; only the message answering this request's id is the
        // reply. Id-less requests are answered by `Pong`.
        let answers = match cmd.request_id() {
            Some(id) => msg.reply_id() == Some(id),
            None => matches!(msg, ProcessToService::Pong) && matches!(cmd, ServiceToProcess::Ping),
        };
        if answers {
            return Ok(msg);
        }
    }
}
