//! One request and its correlated reply over a fresh VM-owner IPC connection.
use super::*;

#[tracing::instrument(skip_all, fields(cmd = ?std::mem::discriminant(&cmd), timeout_secs = ?timeout_secs))]
pub(crate) async fn send_ipc_command(
    state: &ServiceState,
    uds_path: &std::path::Path,
    cmd: ServiceToProcess,
    timeout_secs: Option<u64>,
) -> Result<ProcessToService, String> {
    let owner = owner_connection::OwnerConnection::acquire(state, uds_path)?;
    send_owner_command(state, &owner, cmd, timeout_secs).await
}

pub(crate) async fn send_owner_command(
    state: &ServiceState,
    owner: &owner_connection::OwnerConnection,
    cmd: ServiceToProcess,
    timeout_secs: Option<u64>,
) -> Result<ProcessToService, String> {
    let (tx, rx) = owner.open(state, "capsem-service", false).await?;

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
            owner.validate(state, false)?;
            return Ok(msg);
        }
    }
}
