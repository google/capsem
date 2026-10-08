//! Broker-private verified connection ownership; no session grant is implied.
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Instant;

use super::{GoogleOAuthClient, OAuthIdentityError, OAuthProviderError, OAuthTokenError, OAuthTokens, Secret};

mod storage;
pub use storage::OAuthConnectionStorage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoogleConnectionState {
    Connected,
    ReauthRequired,
    Disconnecting,
    Disconnected,
    Degraded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GoogleConnectionStatus {
    pub state: GoogleConnectionState,
    pub revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoogleConnectionError {
    InvalidStorage,
    StorageUnavailable,
    InvalidRecord,
    AlreadyExists,
    Unavailable,
    RevisionChanged,
    RevisionExhausted,
    Identity(OAuthIdentityError),
    Token(OAuthTokenError),
}
impl fmt::Display for GoogleConnectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidStorage => "invalid OAuth connection storage",
            Self::StorageUnavailable => "OAuth connection storage unavailable",
            Self::InvalidRecord => "invalid OAuth connection record",
            Self::AlreadyExists => "OAuth connection already exists",
            Self::Unavailable => "OAuth connection unavailable",
            Self::RevisionChanged => "OAuth connection revision changed",
            Self::RevisionExhausted => "OAuth connection revision exhausted",
            Self::Identity(_) => "OAuth connection identity verification failed",
            Self::Token(_) => "OAuth connection provider operation failed",
        })
    }
}
impl std::error::Error for GoogleConnectionError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoogleRevocationOutcome {
    Succeeded,
    Failed(OAuthTokenError),
    NotNeeded,
}

#[derive(Clone)]
struct Record {
    client_id: Arc<str>,
    subject: Arc<Secret>,
    revision: u64,
    state: GoogleConnectionState,
    tokens: Option<Arc<OAuthTokens>>,
}
impl Record {
    fn advance(&self) -> Result<Self, GoogleConnectionError> {
        let mut next = self.clone();
        next.revision = self
            .revision
            .checked_add(1)
            .filter(|value| *value < (1_u64 << 53))
            .ok_or(GoogleConnectionError::RevisionExhausted)?;
        Ok(next)
    }
}
struct Core {
    client: Arc<GoogleOAuthClient>,
    storage: OAuthConnectionStorage,
    data: Mutex<Record>,
    writer: Mutex<()>,
    denied: AtomicBool,
    denial_generation: AtomicU64,
    lifecycle: tokio::sync::Mutex<()>,
    refresh: tokio::sync::Mutex<()>,
    refresh_sequence: AtomicU64,
    last_refresh_error: Mutex<Option<GoogleConnectionError>>,
}

#[derive(Clone)]
pub struct GoogleConnection(Arc<Core>);
impl fmt::Debug for GoogleConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("GoogleConnection").field(&self.status()).finish()
    }
}

/// Host-only access admission. A callback cannot borrow material beyond the
/// admission and must be short and synchronous. Session policy belongs above it.
pub struct GoogleAccessLease {
    owner: Weak<Core>,
    revision: u64,
    tokens: Arc<OAuthTokens>,
}
impl fmt::Debug for GoogleAccessLease {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GoogleAccessLease")
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}
impl GoogleAccessLease {
    pub fn with_token<T>(&self, now: Instant, use_token: impl FnOnce(&str) -> T) -> Result<T, GoogleConnectionError> {
        let owner = self.owner.upgrade().ok_or(GoogleConnectionError::Unavailable)?;
        let record = owner.data.lock().map_err(|_| GoogleConnectionError::Unavailable)?;
        if owner.denied.load(Ordering::Acquire)
            || record.state != GoogleConnectionState::Connected
            || record.revision != self.revision
        {
            return Err(GoogleConnectionError::Unavailable);
        }
        let token = self
            .tokens
            .access_token(now)
            .ok_or(GoogleConnectionError::Unavailable)?;
        let result = use_token(token);
        drop(record);
        Ok(result)
    }
}

impl GoogleOAuthClient {
    /// A connection is constructed only from this exact verified exchange.
    /// No registration, profile, capability or session approval is discovered.
    pub async fn open_connection(
        self: &Arc<Self>,
        mut tokens: OAuthTokens,
        storage: OAuthConnectionStorage,
    ) -> Result<GoogleConnection, GoogleConnectionError> {
        let identity = self
            .verify_identity(&tokens, None)
            .await
            .map_err(GoogleConnectionError::Identity)?;
        tokens.id_token = None;
        tokens.expected_nonce = None;
        let record = Record {
            client_id: Arc::from(tokens.client_id.as_str()),
            subject: Arc::new(Secret::new(identity.subject().to_owned())),
            revision: 1,
            state: GoogleConnectionState::Connected,
            tokens: Some(Arc::new(tokens)),
        };
        let client = Arc::clone(self);
        tokio::task::spawn_blocking(move || {
            if storage.read(&client)?.is_some() {
                return Err(GoogleConnectionError::AlreadyExists);
            }
            storage.write(&record)?;
            Ok(GoogleConnection::from_record(client, storage, record))
        })
        .await
        .map_err(|_| GoogleConnectionError::StorageUnavailable)?
    }
}

impl GoogleConnection {
    fn from_record(client: Arc<GoogleOAuthClient>, storage: OAuthConnectionStorage, record: Record) -> Self {
        let denied = record.state != GoogleConnectionState::Connected;
        Self(Arc::new(Core {
            client,
            storage,
            data: Mutex::new(record),
            writer: Mutex::new(()),
            denied: AtomicBool::new(denied),
            denial_generation: AtomicU64::new(0),
            lifecycle: tokio::sync::Mutex::new(()),
            refresh: tokio::sync::Mutex::new(()),
            refresh_sequence: AtomicU64::new(0),
            last_refresh_error: Mutex::new(None),
        }))
    }

    pub async fn restore(
        client: Arc<GoogleOAuthClient>,
        storage: OAuthConnectionStorage,
    ) -> Result<Self, GoogleConnectionError> {
        tokio::task::spawn_blocking(move || {
            let record = storage.read(&client)?.ok_or(GoogleConnectionError::InvalidRecord)?;
            Ok(Self::from_record(client, storage, record))
        })
        .await
        .map_err(|_| GoogleConnectionError::StorageUnavailable)?
    }

    pub fn status(&self) -> GoogleConnectionStatus {
        self.0
            .data
            .lock()
            .map(|record| GoogleConnectionStatus {
                revision: record.revision,
                state: if self.0.denied.load(Ordering::Acquire) && record.state == GoogleConnectionState::Connected {
                    GoogleConnectionState::Disconnecting
                } else {
                    record.state
                },
            })
            .unwrap_or(GoogleConnectionStatus {
                revision: 0,
                state: GoogleConnectionState::Degraded,
            })
    }

    fn snapshot(&self) -> Result<Record, GoogleConnectionError> {
        self.0
            .data
            .lock()
            .map(|record| record.clone())
            .map_err(|_| GoogleConnectionError::Unavailable)
    }

    fn active_snapshot(&self) -> Result<Record, GoogleConnectionError> {
        let record = self.snapshot()?;
        if self.0.denied.load(Ordering::Acquire) || record.state != GoogleConnectionState::Connected {
            return Err(GoogleConnectionError::Unavailable);
        }
        Ok(record)
    }

    fn lease(&self, record: Record) -> Result<GoogleAccessLease, GoogleConnectionError> {
        Ok(GoogleAccessLease {
            owner: Arc::downgrade(&self.0),
            revision: record.revision,
            tokens: record.tokens.ok_or(GoogleConnectionError::Unavailable)?,
        })
    }

    pub async fn access(&self, now: Instant) -> Result<GoogleAccessLease, GoogleConnectionError> {
        let sequence = self.0.refresh_sequence.load(Ordering::Acquire);
        let record = self.active_snapshot()?;
        if record
            .tokens
            .as_ref()
            .is_some_and(|tokens| tokens.access_token(now).is_some())
        {
            return self.lease(record);
        }
        let _refresh = self.0.refresh.lock().await;
        let current = self.active_snapshot()?;
        if current
            .tokens
            .as_ref()
            .is_some_and(|tokens| tokens.access_token(now).is_some())
        {
            return self.lease(current);
        }
        if self.0.refresh_sequence.load(Ordering::Acquire) != sequence {
            return Err(self
                .0
                .last_refresh_error
                .lock()
                .map_err(|_| GoogleConnectionError::Unavailable)?
                .unwrap_or(GoogleConnectionError::Unavailable));
        }
        let previous = current.tokens.as_ref().ok_or(GoogleConnectionError::Unavailable)?;
        let outcome = match self.0.client.refresh(previous).await {
            Ok(mut tokens) => {
                tokens.id_token = None;
                tokens.expected_nonce = None;
                let mut next = current.advance()?;
                next.tokens = Some(Arc::new(tokens));
                self.publish(current.revision, next).await
            }
            Err(error) => {
                if matches!(
                    error,
                    OAuthTokenError::ReauthorizationRequired
                        | OAuthTokenError::MissingRefreshToken
                        | OAuthTokenError::ProviderRejected {
                            code: Some(OAuthProviderError::InvalidGrant),
                            ..
                        }
                ) {
                    let mut next = current.advance()?;
                    next.state = GoogleConnectionState::ReauthRequired;
                    self.publish(current.revision, next).await?;
                }
                Err(GoogleConnectionError::Token(error))
            }
        };
        *self
            .0
            .last_refresh_error
            .lock()
            .map_err(|_| GoogleConnectionError::Unavailable)? = outcome.as_ref().err().copied();
        self.0.refresh_sequence.fetch_add(1, Ordering::Release);
        outcome?;
        self.lease(self.active_snapshot()?)
    }

    async fn publish(&self, expected: u64, next: Record) -> Result<(), GoogleConnectionError> {
        let owner = Arc::clone(&self.0);
        tokio::task::spawn_blocking(move || {
            let _writer = owner
                .writer
                .lock()
                .map_err(|_| GoogleConnectionError::StorageUnavailable)?;
            if owner.denied.load(Ordering::Acquire)
                || owner
                    .data
                    .lock()
                    .map_err(|_| GoogleConnectionError::Unavailable)?
                    .revision
                    != expected
            {
                return Err(GoogleConnectionError::RevisionChanged);
            }
            if let Err(error) = owner.storage.write(&next) {
                let mut data = owner.data.lock().map_err(|_| GoogleConnectionError::Unavailable)?;
                if data.revision == expected && !owner.denied.load(Ordering::Acquire) {
                    data.state = GoogleConnectionState::Degraded;
                    owner.denied.store(true, Ordering::Release);
                }
                drop(data);
                return Err(error);
            }
            let mut data = owner.data.lock().map_err(|_| GoogleConnectionError::Unavailable)?;
            if owner.denied.load(Ordering::Acquire) || data.revision != expected {
                return Err(GoogleConnectionError::RevisionChanged);
            }
            *data = next;
            drop(data);
            Ok(())
        })
        .await
        .map_err(|_| GoogleConnectionError::StorageUnavailable)?
    }

    /// Deny new access immediately; completed local disconnection is durable.
    /// Provider acknowledgement is reported separately and is not a VM fence.
    pub async fn disconnect(&self) -> Result<GoogleRevocationOutcome, GoogleConnectionError> {
        self.0.denial_generation.fetch_add(1, Ordering::AcqRel);
        self.0.denied.store(true, Ordering::Release);
        let _lifecycle = self.0.lifecycle.lock().await;
        let owner = Arc::clone(&self.0);
        let tokens = tokio::task::spawn_blocking(move || {
            let _writer = owner
                .writer
                .lock()
                .map_err(|_| GoogleConnectionError::StorageUnavailable)?;
            let current = owner
                .data
                .lock()
                .map_err(|_| GoogleConnectionError::Unavailable)?
                .clone();
            if current.state == GoogleConnectionState::Disconnected {
                return Ok(None);
            }
            let mut next = current.advance()?;
            next.state = GoogleConnectionState::Disconnected;
            next.tokens = None;
            owner.storage.write(&next)?;
            *owner.data.lock().map_err(|_| GoogleConnectionError::Unavailable)? = next;
            Ok(current.tokens)
        })
        .await
        .map_err(|_| GoogleConnectionError::StorageUnavailable)??;
        match tokens {
            Some(tokens) => Ok(match self.0.client.revoke(&tokens).await {
                Ok(()) => GoogleRevocationOutcome::Succeeded,
                Err(error) => GoogleRevocationOutcome::Failed(error),
            }),
            None => Ok(GoogleRevocationOutcome::NotNeeded),
        }
    }

    /// Reauthorization must prove the original account again. Old access
    /// leases remain invalid; higher-level session grants are never restored.
    pub async fn reconnect(&self, mut tokens: OAuthTokens) -> Result<(), GoogleConnectionError> {
        let denial = self.0.denial_generation.load(Ordering::Acquire);
        let _lifecycle = self.0.lifecycle.lock().await;
        let current = self.snapshot()?;
        if self.0.denial_generation.load(Ordering::Acquire) != denial
            || (current.state == GoogleConnectionState::Connected && self.0.denied.load(Ordering::Acquire))
        {
            return Err(GoogleConnectionError::RevisionChanged);
        }
        self.0
            .client
            .verify_identity(&tokens, Some(current.subject.expose()))
            .await
            .map_err(GoogleConnectionError::Identity)?;
        tokens.id_token = None;
        tokens.expected_nonce = None;
        let mut next = current.advance()?;
        next.state = GoogleConnectionState::Connected;
        next.tokens = Some(Arc::new(tokens));
        let owner = Arc::clone(&self.0);
        tokio::task::spawn_blocking(move || {
            let _writer = owner
                .writer
                .lock()
                .map_err(|_| GoogleConnectionError::StorageUnavailable)?;
            if owner.denial_generation.load(Ordering::Acquire) != denial
                || owner
                    .data
                    .lock()
                    .map_err(|_| GoogleConnectionError::Unavailable)?
                    .revision
                    != current.revision
            {
                return Err(GoogleConnectionError::RevisionChanged);
            }
            owner.storage.write(&next)?;
            let mut data = owner.data.lock().map_err(|_| GoogleConnectionError::Unavailable)?;
            if owner.denial_generation.load(Ordering::Acquire) != denial || data.revision != current.revision {
                return Err(GoogleConnectionError::RevisionChanged);
            }
            *data = next;
            owner.denied.store(false, Ordering::Release);
            drop(data);
            Ok(())
        })
        .await
        .map_err(|_| GoogleConnectionError::StorageUnavailable)?
    }
}

#[cfg(test)]
mod tests;
