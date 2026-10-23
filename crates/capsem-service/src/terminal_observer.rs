//! Lightweight session metadata for list/tray consumers, even with no UI.
//! Bound to one owner generation and revoked with that owner. No screen or
//! ledger projection is retained in the service.
use super::*;

pub(super) fn start(state: Arc<ServiceState>, id: String, generation: uuid::Uuid) {
    tokio::spawn(async move {
        let observed: Result<(), String> = async {
            let path = {
                let instances = state.instances.lock().unwrap();
                instances
                    .get(&id)
                    .filter(|instance| instance.generation == generation)
                    .map(|instance| instance.uds_path.clone())
            }
            .ok_or_else(|| "terminal owner no longer registered".to_string())?;
            let grant = crate::owner_connection::OwnerConnection::acquire(&state, &path)?;
            tokio::select! {
                _ = grant.revoked() => return Ok(()),
                ready = wait_for_vm_ready(&path, 30, Some(&state), Some(&id)) => ready?,
            }
            let (_tx, rx) = grant.open(&state, "capsem-terminal-observer", false).await?;
            loop {
                let message = tokio::select! {
                    _ = grant.revoked() => return Ok(()),
                    message = rx.recv() => message.map_err(|error| error.to_string())?,
                };
                if let ProcessToService::TerminalSummary { summary } = message {
                    summary.validate().map_err(str::to_owned)?;
                    let mut instances = state.instances.lock().unwrap();
                    let Some(instance) = instances
                        .get_mut(&id)
                        .filter(|instance| instance.generation == generation)
                    else {
                        return Ok(());
                    };
                    if instance
                        .terminal
                        .as_ref()
                        .is_none_or(|current| summary.revision >= current.revision)
                    {
                        instance.terminal = Some(summary);
                    }
                    drop(instances);
                } else if let ProcessToService::BrowserRequest { mut request } = message {
                    request.url = capsem_proto::browser::browser_url(&request.url).map_err(str::to_owned)?;
                    let mut instances = state.instances.lock().unwrap();
                    let Some(instance) = instances
                        .get_mut(&id)
                        .filter(|instance| instance.generation == generation)
                    else {
                        return Ok(());
                    };
                    instance.observe_browser(request);
                    drop(instances);
                } else if let ProcessToService::BrowserRequestCleared { request_id } = message {
                    let mut instances = state.instances.lock().unwrap();
                    let Some(instance) = instances
                        .get_mut(&id)
                        .filter(|instance| instance.generation == generation)
                    else {
                        return Ok(());
                    };
                    instance.clear_browser(request_id);
                    drop(instances);
                }
            }
        }
        .await;
        if let Err(error) = observed {
            tracing::debug!(%id, %error, "terminal metadata observer ended");
        }
    });
}
