//! Restartable, bounded provider retirement owned by the durable disconnect intent.
use super::*;

impl ConnectionBroker {
    /// Inventory reads never schedule provider work. Startup and maintenance
    /// resume only explicit durable disconnect intent, never enrollment.
    pub(super) async fn resume_retirements(&self) -> BrokerResult<()> {
        let mut state = self.0.state.lock().await;
        let ids: Vec<_> = state
            .connections
            .iter()
            .filter(|(_, entry)| entry.record.retiring)
            .map(|(id, _)| id.clone())
            .collect();
        let mut changed = false;
        for id in &ids {
            changed |= self.retire_connection_grants(&mut state, id)? > 0;
        }
        if changed {
            self.0.store.write(state.snapshot()).await?;
        }
        for id in ids {
            self.start_retirement(&mut state, &id)?;
        }
        drop(state);
        Ok(())
    }

    pub(super) fn start_retirement(&self, state: &mut State, id: &str) -> BrokerResult<()> {
        let connection = state
            .connections
            .get(id)
            .and_then(|entry| entry.connection.clone())
            .ok_or(api::CredentialErrorCode::ConnectionUnavailable)?;
        if !state.jobs.get(id).is_some_and(|job| !job.is_finished()) {
            let owner = Arc::downgrade(&self.0);
            let id = id.to_owned();
            let timeout = self.0.limits.retirement_timeout;
            let key = id.clone();
            let job = tokio::spawn(async move {
                let result = tokio::time::timeout(timeout, connection.disconnect())
                    .await
                    .map_err(|_| api::CredentialErrorCode::ProviderTimeout)
                    .and_then(|result| result.map_err(connection_error));
                if let Some(owner) = owner.upgrade() {
                    let mut state = owner.state.lock().await;
                    if let Some(entry) = state.connections.get_mut(&id) {
                        entry.record.error_code = match result {
                            Ok(
                                crate::GoogleRevocationOutcome::Succeeded | crate::GoogleRevocationOutcome::NotNeeded,
                            ) => {
                                entry.record.retiring = false;
                                None
                            }
                            Ok(crate::GoogleRevocationOutcome::Failed(_)) => {
                                Some(api::CredentialErrorCode::ProviderRejected)
                            }
                            Err(error) => Some(error),
                        };
                        if owner.store.write(state.snapshot()).await.is_err() {
                            let record = &mut state.connections.get_mut(&id).expect("retained connection").record;
                            record.retiring = true;
                            record.error_code = Some(api::CredentialErrorCode::StorageCommitFailed);
                        }
                    }
                    state.jobs.remove(&id);
                }
            });
            state.jobs.insert(key, job);
        }
        Ok(())
    }
}
