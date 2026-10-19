//! One request and its correlated reply over a fresh VM-owner IPC connection.
use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum IpcCommandError {
    Timeout { timeout_secs: u64 },
    Failed(String),
}

impl std::fmt::Display for IpcCommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout { timeout_secs } => write!(f, "IPC command timed out after {timeout_secs}s"),
            Self::Failed(msg) => f.write_str(msg),
        }
    }
}

impl From<String> for IpcCommandError {
    fn from(error: String) -> Self {
        Self::Failed(error)
    }
}

impl From<IpcCommandError> for String {
    fn from(error: IpcCommandError) -> Self {
        error.to_string()
    }
}

impl IpcCommandError {
    pub(crate) fn into_exec_app_error(self, prefix_failed: bool) -> AppError {
        match self {
            Self::Timeout { timeout_secs } => AppError::exec_timeout(format!("exec failed: {self}"), timeout_secs),
            Self::Failed(msg) if prefix_failed => {
                AppError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("exec failed: {msg}"))
            }
            Self::Failed(msg) => AppError::new(StatusCode::INTERNAL_SERVER_ERROR, msg),
        }
    }

    pub(crate) fn into_internal_app_error(self) -> AppError {
        AppError::new(StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
    }
}

#[tracing::instrument(skip_all, fields(cmd = ?std::mem::discriminant(&cmd), timeout_secs = ?timeout_secs))]
pub(crate) async fn send_ipc_command(
    state: &ServiceState,
    uds_path: &std::path::Path,
    cmd: ServiceToProcess,
    timeout_secs: Option<u64>,
) -> Result<ProcessToService, IpcCommandError> {
    let owner = owner_connection::OwnerConnection::acquire(state, uds_path)?;
    send_owner_command(state, &owner, cmd, timeout_secs).await
}

pub(crate) async fn send_owner_command(
    state: &ServiceState,
    owner: &owner_connection::OwnerConnection,
    cmd: ServiceToProcess,
    timeout_secs: Option<u64>,
) -> Result<ProcessToService, IpcCommandError> {
    let (tx, rx) = owner.open(state, "capsem-service", false).await?;

    tx.send(cmd.clone())
        .await
        .map_err(|e| IpcCommandError::Failed(format!("failed to send IPC command: {e}")))?;

    let deadline = timeout_secs.map(|secs| tokio::time::Instant::now() + std::time::Duration::from_secs(secs));
    loop {
        let msg = match deadline {
            Some(deadline) => match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Ok(msg)) => msg,
                Ok(Err(e)) => {
                    error!(?e, "IPC receive error");
                    return Err(IpcCommandError::Failed(format!("IPC connection closed: {e}")));
                }
                Err(_) => {
                    let timeout_secs = timeout_secs.unwrap_or_default();
                    return Err(IpcCommandError::Timeout { timeout_secs });
                }
            },
            None => match rx.recv().await {
                Ok(msg) => msg,
                Err(e) => {
                    error!(?e, "IPC receive error");
                    return Err(IpcCommandError::Failed(format!("IPC connection closed: {e}")));
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

#[cfg(test)]
mod tests;
