//! Bounded redacted authority metadata; credential material belongs to the provider.
use super::*;
use capsem_foundation::unix::{contained::ContainedDir, fs, lock};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::PathBuf;

pub enum BrokerStorage {
    Memory,
    File { path: PathBuf, max_bytes: usize },
}
pub(super) struct Store {
    file: Option<(PathBuf, lock::FileLock)>,
    max_bytes: usize,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ConnectionRecord {
    pub id: String,
    pub scope_profile_id: String,
    pub retiring: bool,
    pub created_at: SystemTime,
    pub error_code: Option<api::CredentialErrorCode>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TransactionRecord {
    pub request: api::AuthorizeGoogleRequest,
    pub transaction: api::AuthorizationTransaction,
    pub target_connection_id: String,
    pub terminal_at: Option<SystemTime>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GrantRecord {
    pub request: api::CreateSessionGrantRequest,
    pub summary: api::SessionGrantSummary,
    pub consent_generation: u64,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Snapshot {
    pub version: u32,
    pub connections: Vec<ConnectionRecord>,
    pub transactions: Vec<TransactionRecord>,
    pub grants: Vec<GrantRecord>,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            version: 1,
            connections: vec![],
            transactions: vec![],
            grants: vec![],
        }
    }
}
impl Snapshot {
    pub(super) fn validate(&self, limits: BrokerLimits) -> BrokerResult<()> {
        let valid = || -> BrokerResult<()> {
            if self.version != 1
                || self.connections.len() > limits.max_connections
                || self.transactions.len() > limits.max_transactions
                || self.grants.len() > limits.max_grants
            {
                return Err(api::CredentialErrorCode::StorageUnavailable);
            }
            let mut connections = BTreeSet::new();
            for record in &self.connections {
                uuid(&record.id)?;
                if !catalog_id(&record.scope_profile_id)
                    || record.created_at > SystemTime::now()
                    || !connections.insert(&record.id)
                {
                    return Err(api::CredentialErrorCode::StorageUnavailable);
                }
            }
            let mut transactions = BTreeSet::new();
            let mut requests = BTreeSet::new();
            for record in &self.transactions {
                uuid(&record.transaction.id)?;
                uuid(&record.request.request_id)?;
                uuid(&record.target_connection_id)?;
                if !catalog_id(&record.request.scope_profile_id)
                    || !transactions.insert(&record.transaction.id)
                    || !requests.insert(&record.request.request_id)
                    || humantime::parse_rfc3339(&record.transaction.expires_at).is_err()
                    || record
                        .transaction
                        .connection_id
                        .as_ref()
                        .is_some_and(|id| id != &record.target_connection_id)
                {
                    return Err(api::CredentialErrorCode::StorageUnavailable);
                }
            }
            let mut grants = BTreeSet::new();
            let mut grant_requests = BTreeSet::new();
            for record in &self.grants {
                uuid(&record.summary.id)?;
                uuid(&record.summary.session_id)?;
                uuid(&record.summary.owner_generation)?;
                uuid(&record.request.request_id)?;
                if !connections.contains(&record.summary.connection_id)
                    || !grants.insert(&record.summary.id)
                    || !grant_requests.insert(&record.request.request_id)
                    || record.consent_generation == 0
                    || record.consent_generation >= 1_u64 << 53
                    || record.request.expected_owner_generation != record.summary.owner_generation
                    || record.request.connection_id != record.summary.connection_id
                    || record.request.capability_ids != record.summary.capability_ids
                    || humantime::parse_rfc3339(&record.summary.expires_at).is_err()
                    || humantime::parse_rfc3339(&record.summary.created_at).is_err()
                {
                    return Err(api::CredentialErrorCode::StorageUnavailable);
                }
            }
            Ok(())
        };
        valid().map_err(|_| api::CredentialErrorCode::StorageUnavailable)
    }
}
impl Store {
    pub(super) async fn open(storage: BrokerStorage) -> BrokerResult<(Self, Snapshot)> {
        tokio::task::spawn_blocking(move || {
            let BrokerStorage::File { path, max_bytes } = storage else {
                return Ok((
                    Self {
                        file: None,
                        max_bytes: usize::MAX,
                    },
                    Snapshot::default(),
                ));
            };
            if max_bytes == 0 || max_bytes == usize::MAX {
                return Err(api::CredentialErrorCode::ConfigurationRequired);
            }
            let parent = path.parent().ok_or(api::CredentialErrorCode::StorageUnavailable)?;
            fs::ensure_private_dir(parent).map_err(|_| api::CredentialErrorCode::StorageUnavailable)?;
            let name = path.file_name().ok_or(api::CredentialErrorCode::StorageUnavailable)?;
            let owner =
                match lock::try_acquire(&parent.join(".connection-broker.owner.lock"), lock::LockMode::Exclusive)
                    .map_err(|_| api::CredentialErrorCode::StorageUnavailable)?
                {
                    lock::LockAttempt::Acquired(owner) => owner,
                    lock::LockAttempt::Contended => return Err(api::CredentialErrorCode::StorageUnavailable),
                };
            let root = ContainedDir::open_root(parent).map_err(|_| api::CredentialErrorCode::StorageUnavailable)?;
            root.validate_private()
                .map_err(|_| api::CredentialErrorCode::StorageUnavailable)?;
            let snapshot = match root.open_existing_private_append(name) {
                Ok(file) => {
                    let mut bytes = vec![];
                    file.take(max_bytes as u64 + 1)
                        .read_to_end(&mut bytes)
                        .map_err(|_| api::CredentialErrorCode::StorageUnavailable)?;
                    if bytes.len() > max_bytes {
                        return Err(api::CredentialErrorCode::StorageUnavailable);
                    }
                    serde_json::from_slice(&bytes).map_err(|_| api::CredentialErrorCode::StorageUnavailable)?
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Snapshot::default(),
                Err(_) => return Err(api::CredentialErrorCode::StorageUnavailable),
            };
            Ok((
                Self {
                    file: Some((path, owner)),
                    max_bytes,
                },
                snapshot,
            ))
        })
        .await
        .map_err(|_| api::CredentialErrorCode::StorageUnavailable)?
    }
    pub(super) async fn write(&self, snapshot: Snapshot) -> BrokerResult<()> {
        let Some((path, _)) = &self.file else {
            return Ok(());
        };
        let path = path.clone();
        let max_bytes = self.max_bytes;
        tokio::task::spawn_blocking(move || {
            let bytes = serde_json::to_vec(&snapshot).map_err(|_| api::CredentialErrorCode::StorageCommitFailed)?;
            if bytes.len() > max_bytes {
                return Err(api::CredentialErrorCode::StorageCommitFailed);
            }
            fs::atomic_write_private(&path, &bytes).map_err(|_| api::CredentialErrorCode::StorageCommitFailed)
        })
        .await
        .map_err(|_| api::CredentialErrorCode::StorageCommitFailed)?
    }
}
