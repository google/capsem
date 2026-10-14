//! Finite retry history. Active authority and pending jobs are never evicted.
use super::*;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BrokerPruned {
    pub transactions: usize,
    pub grants: usize,
    pub connections: usize,
}
impl ConnectionBroker {
    /// Service maintenance owns this operation; inventory reads do no I/O.
    pub async fn prune(&self) -> BrokerResult<BrokerPruned> {
        self.prune_at(SystemTime::now()).await
    }

    pub(super) async fn prune_at(&self, now: SystemTime) -> BrokerResult<BrokerPruned> {
        let Some(cutoff) = now.checked_sub(self.0.limits.retention_lifetime) else {
            return Ok(BrokerPruned::default());
        };
        let mut state = self.0.state.lock().await;
        let mut changed = false;
        let mut remove = vec![];
        for (id, entry) in &mut state.grants {
            let expires = humantime::parse_rfc3339(&entry.record.summary.expires_at)
                .map_err(|_| api::CredentialErrorCode::StorageUnavailable)?;
            if entry.record.summary.state == api::GrantState::Active && expires <= now {
                if let Some(handle) = entry.handle.take() {
                    self.0
                        .authority
                        .revoke(handle)
                        .map_err(|_| api::CredentialErrorCode::GrantUnavailable)?;
                }
                entry.record.summary.state = api::GrantState::Expired;
                entry.record.summary.runtime_bound = false;
                entry.record.summary.revision = next_revision(entry.record.summary.revision)?;
                changed = true;
            }
            let terminal = match entry.record.summary.state {
                api::GrantState::Active => continue,
                api::GrantState::Expired => expires,
                api::GrantState::Revoked => humantime::parse_rfc3339(
                    entry
                        .record
                        .summary
                        .revoked_at
                        .as_ref()
                        .ok_or(api::CredentialErrorCode::StorageUnavailable)?,
                )
                .map_err(|_| api::CredentialErrorCode::StorageUnavailable)?,
            };
            if terminal <= cutoff {
                remove.push(id.clone());
            }
        }
        let mut result = BrokerPruned {
            grants: remove.len(),
            ..Default::default()
        };
        for id in remove {
            state.grants.remove(&id);
        }
        let mut remove = vec![];
        for (id, entry) in &state.transactions {
            if entry.record.transaction.state == api::TransactionState::Pending || state.jobs.contains_key(id) {
                continue;
            }
            // Old private records predate terminal_at. Their expiry is a
            // conservative anchor, preserving at least the promised window.
            let terminal = entry.record.terminal_at.unwrap_or(
                humantime::parse_rfc3339(&entry.record.transaction.expires_at)
                    .map_err(|_| api::CredentialErrorCode::StorageUnavailable)?,
            );
            if terminal <= cutoff {
                remove.push(id.clone());
            }
        }
        result.transactions = remove.len();
        for id in remove {
            state.transactions.remove(&id);
        }
        let mut remove = vec![];
        for (id, entry) in &state.connections {
            if entry.record.retiring
                || state.jobs.contains_key(id)
                || state
                    .transactions
                    .values()
                    .any(|transaction| &transaction.record.target_connection_id == id)
                || state
                    .grants
                    .values()
                    .any(|grant| &grant.record.summary.connection_id == id)
            {
                continue;
            }
            let Some(connection) = &entry.connection else {
                continue;
            };
            let metadata = connection.metadata().map_err(connection_error)?;
            if metadata.status.state != crate::GoogleConnectionState::Disconnected
                || !matches!(
                    metadata.status.revocation,
                    crate::GoogleRevocationState::Succeeded | crate::GoogleRevocationState::NotNeeded
                )
            {
                continue;
            }
            let terminal = UNIX_EPOCH
                .checked_add(Duration::from_millis(metadata.updated_at_ms))
                .ok_or(api::CredentialErrorCode::StorageUnavailable)?;
            if terminal <= cutoff {
                remove.push(id.clone());
            }
        }
        result.connections = remove.len();
        for id in remove {
            state.connections.remove(&id);
        }
        if changed || result != BrokerPruned::default() {
            self.0.store.write(state.snapshot()).await?;
        }
        drop(state);
        Ok(result)
    }
}
