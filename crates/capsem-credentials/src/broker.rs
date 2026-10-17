//! One host owner for explicit enrollment, redacted connection inventory and
//! finite session approvals. Transport/authentication and policy live above it.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use capsem_api as api;
use capsem_crypto::secrets::Zeroizing;
use tokio::sync::Mutex;

use crate::{GoogleConnection, GrantAuthority, GrantCapability, GrantHandle, GrantSession};

mod enrollment;
mod policy;
mod retention;
mod retirement;
use policy::{label, normalize_grant_request, normalize_profile, scope};
pub use retention::BrokerPruned;
mod sessions;
pub use sessions::SessionBinding;
mod approvals;
pub use approvals::ApprovalOwner;
mod storage;
pub use storage::BrokerStorage;
use storage::{ConnectionRecord, GrantRecord, Snapshot, Store, TransactionRecord};

pub type BrokerResult<T> = Result<T, api::CredentialErrorCode>;
pub type EnrollmentFuture<T> = Pin<Box<dyn Future<Output = BrokerResult<T>> + Send + 'static>>;

/// A trusted enrollment adapter owns the native browser/callback/provider
/// lifecycle. Dropping its completion must close pending callbacks and retire
/// uncommitted credential material. No request can choose a client or endpoint.
pub trait EnrollmentProvider: Send + Sync {
    fn begin(&self, connection_id: String, reconnect: Option<GoogleConnection>) -> EnrollmentFuture<Enrollment>;
    fn restore(&self, connection_id: String) -> EnrollmentFuture<GoogleConnection>;
}

pub struct Enrollment {
    /// Trusted handoff only. The broker never logs or persists this URL.
    pub authorization_url: Option<Zeroizing<String>>,
    pub completion: EnrollmentFuture<GoogleConnection>,
}

pub struct BrokerProfile {
    summary: api::ScopeProfileSummary,
    capabilities: BTreeMap<String, GrantCapability>,
    provider: Option<Arc<dyn EnrollmentProvider>>,
}

impl BrokerProfile {
    pub fn unconfigured(mut summary: api::ScopeProfileSummary) -> BrokerResult<Self> {
        let limits = BrokerLimits::bundled()?;
        normalize_profile(&mut summary, limits)?;
        if summary.enabled || summary.capabilities.iter().any(|capability| capability.supported) {
            return Err(api::CredentialErrorCode::ConfigurationRequired);
        }
        Self::validate(&summary, limits)?;
        Ok(Self {
            summary,
            capabilities: BTreeMap::new(),
            provider: None,
        })
    }

    /// Trusted host configuration must establish consumer support independently
    /// of consent. Supplying an adapter is not itself a native-vendor proof.
    pub fn configured(
        mut summary: api::ScopeProfileSummary,
        capabilities: Vec<GrantCapability>,
        provider: Arc<dyn EnrollmentProvider>,
    ) -> BrokerResult<Self> {
        let limits = BrokerLimits::bundled()?;
        normalize_profile(&mut summary, limits)?;
        Self::validate(&summary, limits)?;
        let mut configured = BTreeMap::new();
        for capability in capabilities {
            if configured.insert(capability.id().to_owned(), capability).is_some() {
                return Err(api::CredentialErrorCode::UnsupportedCapability);
            }
        }
        // This lane owns the existing registered Google token protocol. Native
        // enrollment must delegate refresh/custody to its supported official
        // component; wrapping vendor material in this lane is not that adapter.
        if summary.enrollment_kind != api::EnrollmentKind::OwnedOauthClient {
            return Err(api::CredentialErrorCode::UnsupportedCapability);
        }
        if !summary.enabled
            || summary.capabilities.iter().any(|capability| {
                capability.supported != configured.contains_key(&capability.id)
                    || configured.get(&capability.id).is_some_and(|configured| {
                        configured.required_scopes() != &capability.required_scopes.iter().cloned().collect()
                    })
            })
            || configured
                .keys()
                .any(|id| !summary.capabilities.iter().any(|capability| &capability.id == id))
        {
            return Err(api::CredentialErrorCode::UnsupportedCapability);
        }
        Ok(Self {
            summary,
            capabilities: configured,
            provider: Some(provider),
        })
    }

    fn validate(summary: &api::ScopeProfileSummary, limits: BrokerLimits) -> BrokerResult<()> {
        if summary.capabilities.len() > limits.max_capabilities || summary.requested_scopes.len() > limits.max_scopes {
            return Err(api::CredentialErrorCode::ConfigurationRequired);
        }
        let ids: BTreeSet<_> = summary.capabilities.iter().map(|capability| &capability.id).collect();
        if !catalog_id(&summary.id)
            || summary.id.len() > limits.max_identifier_bytes
            || ids.len() != summary.capabilities.len()
            || !label(&summary.label, limits)
            || !label(&summary.application_label, limits)
            || summary.requested_scopes.iter().any(|value| !scope(value, limits))
            || summary.capabilities.iter().any(|capability| {
                !catalog_id(&capability.id)
                    || capability.id.len() > limits.max_identifier_bytes
                    || !label(&capability.label, limits)
                    || !label(&capability.consumer, limits)
                    || capability.required_scopes.len() > limits.max_scopes
                    || capability
                        .required_scopes
                        .iter()
                        .any(|value| !scope(value, limits) || !summary.requested_scopes.contains(value))
            })
        {
            return Err(api::CredentialErrorCode::ConfigurationRequired);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerLimits {
    pub max_connections: usize,
    pub max_transactions: usize,
    pub max_grants: usize,
    pub max_profiles: usize,
    pub max_capabilities: usize,
    pub max_scopes: usize,
    pub max_identifier_bytes: usize,
    pub max_label_bytes: usize,
    pub max_scope_bytes: usize,
    pub max_metadata_bytes: usize,
    #[serde(deserialize_with = "policy::seconds")]
    pub maintenance_interval: Duration,
    #[serde(deserialize_with = "policy::seconds")]
    pub enrollment_lifetime: Duration,
    #[serde(deserialize_with = "policy::seconds")]
    pub grant_lifetime: Duration,
    #[serde(deserialize_with = "policy::seconds")]
    pub retirement_timeout: Duration,
    #[serde(deserialize_with = "policy::seconds")]
    pub retention_lifetime: Duration,
}

struct ConnectionEntry {
    record: ConnectionRecord,
    connection: Option<GoogleConnection>,
}
struct TransactionEntry {
    record: TransactionRecord,
    url: Option<Zeroizing<String>>,
}
struct GrantEntry {
    record: GrantRecord,
    handle: Option<GrantHandle>,
    binding: Option<GrantSession>,
}
#[derive(Default)]
struct State {
    connections: BTreeMap<String, ConnectionEntry>,
    transactions: BTreeMap<String, TransactionEntry>,
    grants: BTreeMap<String, GrantEntry>,
    jobs: BTreeMap<String, tokio::task::JoinHandle<()>>,
}
impl State {
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            version: 1,
            connections: self.connections.values().map(|entry| entry.record.clone()).collect(),
            transactions: self.transactions.values().map(|entry| entry.record.clone()).collect(),
            grants: self.grants.values().map(|entry| entry.record.clone()).collect(),
        }
    }
}
impl Drop for State {
    fn drop(&mut self) {
        for job in self.jobs.values() {
            job.abort();
        }
    }
}
struct Inner {
    profiles: BTreeMap<String, BrokerProfile>,
    authority: Arc<GrantAuthority>,
    limits: BrokerLimits,
    store: Arc<Store>,
    state: Mutex<State>,
}

#[derive(Clone)]
pub struct ConnectionBroker(Arc<Inner>);

impl ConnectionBroker {
    /// Only host-configured, qualified capabilities have target contracts.
    pub fn configured_capabilities(&self) -> impl Iterator<Item = &GrantCapability> {
        self.0
            .profiles
            .values()
            .flat_map(|profile| profile.capabilities.values())
    }
    pub async fn open(
        storage: BrokerStorage,
        profiles: Vec<BrokerProfile>,
        authority: Arc<GrantAuthority>,
        limits: BrokerLimits,
    ) -> BrokerResult<Self> {
        limits.validate()?;
        if profiles.len() > limits.max_profiles {
            return Err(api::CredentialErrorCode::ConfigurationRequired);
        }
        let mut catalog = BTreeMap::new();
        for profile in profiles {
            BrokerProfile::validate(&profile.summary, limits)?;
            if catalog.insert(profile.summary.id.clone(), profile).is_some() {
                return Err(api::CredentialErrorCode::ConfigurationRequired);
            }
        }
        let (store, snapshot) = Store::open(storage).await?;
        snapshot.validate(limits)?;
        let mut state = State::default();
        for record in snapshot.connections {
            let connection = if let Some(provider) = catalog
                .get(&record.scope_profile_id)
                .and_then(|profile| profile.provider.as_ref())
            {
                tokio::time::timeout(limits.enrollment_lifetime, provider.restore(record.id.clone()))
                    .await
                    .map_err(|_| api::CredentialErrorCode::ProviderTimeout)??
                    .into()
            } else {
                return Err(api::CredentialErrorCode::ConfigurationRequired);
            };
            state
                .connections
                .insert(record.id.clone(), ConnectionEntry { record, connection });
        }
        for mut record in snapshot.transactions {
            if record.transaction.state == api::TransactionState::Pending {
                record.transaction.state = api::TransactionState::Cancelled;
                record.transaction.error_code = Some(api::CredentialErrorCode::TransactionCancelled);
                record.terminal_at = Some(SystemTime::now());
            }
            state
                .transactions
                .insert(record.transaction.id.clone(), TransactionEntry { record, url: None });
        }
        for mut record in snapshot.grants {
            record.summary.runtime_bound = false;
            state.grants.insert(
                record.summary.id.clone(),
                GrantEntry {
                    record,
                    handle: None,
                    binding: None,
                },
            );
        }
        store.write(state.snapshot()).await?;
        let broker = Self(Arc::new(Inner {
            profiles: catalog,
            authority,
            limits,
            store: Arc::new(store),
            state: Mutex::new(state),
        }));
        broker.resume_retirements().await?;
        Ok(broker)
    }

    pub async fn list(&self) -> BrokerResult<api::ConnectionListResponse> {
        self.list_page(&api::ConnectionListQuery::default()).await
    }

    /// Keyset inventory reads only bounded owner state, never provider or disk.
    pub async fn list_page(&self, query: &api::ConnectionListQuery) -> BrokerResult<api::ConnectionListResponse> {
        let limit = query
            .limit
            .map_or(self.0.limits.max_connections, |value| value as usize);
        if limit == 0 || limit > self.0.limits.max_connections {
            return Err(api::CredentialErrorCode::RequestConflict);
        }
        if let Some(cursor) = &query.cursor {
            uuid(cursor)?;
        }
        let state = self.0.state.lock().await;
        let mut entries = state.connections.range::<str, _>((
            std::ops::Bound::Excluded(query.cursor.as_deref().unwrap_or("")),
            std::ops::Bound::Unbounded,
        ));
        let connections: Vec<_> = entries
            .by_ref()
            .take(limit)
            .map(|(_, entry)| self.summary(entry))
            .collect::<BrokerResult<_>>()?;
        let next_cursor = if entries.next().is_some() {
            connections.last().map(|entry| entry.id.clone())
        } else {
            None
        };
        drop(state);
        Ok(api::ConnectionListResponse {
            connections,
            scope_profiles: self
                .0
                .profiles
                .values()
                .map(|profile| profile.summary.clone())
                .collect(),
            next_cursor,
        })
    }

    pub async fn connection(&self, id: &str) -> BrokerResult<api::ConnectionSummary> {
        self.summary(
            self.0
                .state
                .lock()
                .await
                .connections
                .get(id)
                .ok_or(api::CredentialErrorCode::ConnectionUnavailable)?,
        )
    }

    fn summary(&self, entry: &ConnectionEntry) -> BrokerResult<api::ConnectionSummary> {
        let Some(connection) = &entry.connection else {
            return Err(api::CredentialErrorCode::ConfigurationRequired);
        };
        let metadata = connection.metadata().map_err(connection_error)?;
        let status = metadata.status;
        let state = if entry.record.retiring && status.state != crate::GoogleConnectionState::Disconnected {
            api::ConnectionState::Disconnecting
        } else {
            match status.state {
                crate::GoogleConnectionState::Connected => api::ConnectionState::Connected,
                crate::GoogleConnectionState::ReauthRequired => api::ConnectionState::ReauthRequired,
                crate::GoogleConnectionState::Disconnecting => api::ConnectionState::Disconnecting,
                crate::GoogleConnectionState::Disconnected => api::ConnectionState::Disconnected,
                crate::GoogleConnectionState::Degraded => api::ConnectionState::Degraded,
            }
        };
        let available_capabilities = if state == api::ConnectionState::Connected && !entry.record.retiring {
            self.0
                .profiles
                .get(&entry.record.scope_profile_id)
                .map(|profile| {
                    profile
                        .capabilities
                        .values()
                        .filter(|capability| capability.required_scopes().is_subset(&metadata.granted_scopes))
                        .map(|capability| capability.id().to_owned())
                        .collect()
                })
                .unwrap_or_default()
        } else {
            vec![]
        };
        Ok(api::ConnectionSummary {
            id: entry.record.id.clone(),
            provider: api::ConnectionProvider::Google,
            scope_profile_id: entry.record.scope_profile_id.clone(),
            account_label: None,
            state,
            revision: status.revision,
            granted_scopes: metadata.granted_scopes.into_iter().collect(),
            available_capabilities,
            created_at: timestamp(UNIX_EPOCH + Duration::from_millis(metadata.created_at_ms))?,
            updated_at: timestamp(UNIX_EPOCH + Duration::from_millis(metadata.updated_at_ms))?,
            last_refresh_at: metadata
                .last_refresh_at_ms
                .map(|ms| timestamp(UNIX_EPOCH + Duration::from_millis(ms)))
                .transpose()?,
            provider_revocation: match status.revocation {
                crate::GoogleRevocationState::NotNeeded => api::ProviderRevocationState::NotStarted,
                crate::GoogleRevocationState::Pending => api::ProviderRevocationState::Pending,
                crate::GoogleRevocationState::Failed => api::ProviderRevocationState::Failed,
                crate::GoogleRevocationState::Succeeded => api::ProviderRevocationState::Succeeded,
            },
            error_code: entry.record.error_code,
        })
    }
}

fn catalog_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}
fn uuid(value: &str) -> BrokerResult<uuid::Uuid> {
    let id = uuid::Uuid::parse_str(value).map_err(|_| api::CredentialErrorCode::RequestConflict)?;
    if id.is_nil()
        || id.to_string() != value
        || id.get_variant() != uuid::Variant::RFC4122
        || !(1..=8).contains(&id.get_version_num())
    {
        return Err(api::CredentialErrorCode::RequestConflict);
    }
    Ok(id)
}
fn timestamp(time: SystemTime) -> BrokerResult<String> {
    time.duration_since(UNIX_EPOCH)
        .map_err(|_| api::CredentialErrorCode::StorageUnavailable)?;
    let mut result = String::new();
    std::fmt::write(&mut result, format_args!("{}", humantime::format_rfc3339_millis(time)))
        .map_err(|_| api::CredentialErrorCode::StorageUnavailable)?;
    Ok(result)
}
fn next_revision(value: u64) -> BrokerResult<u64> {
    value
        .checked_add(1)
        .filter(|revision| *revision < 1_u64 << 53)
        .ok_or(api::CredentialErrorCode::StorageUnavailable)
}
fn connection_error(error: crate::GoogleConnectionError) -> api::CredentialErrorCode {
    use api::CredentialErrorCode as Code;
    match error {
        crate::GoogleConnectionError::StorageUnavailable => Code::StorageCommitFailed,
        crate::GoogleConnectionError::InvalidStorage | crate::GoogleConnectionError::InvalidRecord => {
            Code::StorageUnavailable
        }
        crate::GoogleConnectionError::ScopeDenied => Code::ScopeDenied,
        crate::GoogleConnectionError::RevisionChanged => Code::RequestConflict,
        crate::GoogleConnectionError::Token(crate::OAuthTokenError::Timeout) => Code::ProviderTimeout,
        crate::GoogleConnectionError::Token(crate::OAuthTokenError::ReauthorizationRequired) => Code::ReauthRequired,
        crate::GoogleConnectionError::Identity(_) | crate::GoogleConnectionError::Token(_) => Code::ProviderRejected,
        _ => Code::ConnectionUnavailable,
    }
}

#[cfg(test)]
mod tests;
