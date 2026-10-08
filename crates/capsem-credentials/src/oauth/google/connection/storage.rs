//! Temporary owner-only file storage approved by the user; no OS vault prompts.
use std::io::Read;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use capsem_foundation::unix::{
    contained::ContainedDir,
    fs,
    lock::{self, FileLock, LockAttempt, LockMode},
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use super::*;

pub struct OAuthConnectionStorage {
    file: Option<FileStorage>,
    max_bytes: usize,
}
struct FileStorage {
    path: PathBuf,
    _owner: FileLock,
}
impl fmt::Debug for OAuthConnectionStorage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuthConnectionStorage")
            .field("file", &self.file.is_some())
            .finish_non_exhaustive()
    }
}
impl OAuthConnectionStorage {
    pub fn memory(max_bytes: usize) -> Result<Self, GoogleConnectionError> {
        if max_bytes == 0 {
            return Err(GoogleConnectionError::InvalidStorage);
        }
        Ok(Self { file: None, max_bytes })
    }
    pub async fn file(path: PathBuf, max_bytes: usize) -> Result<Self, GoogleConnectionError> {
        if max_bytes == 0 {
            return Err(GoogleConnectionError::InvalidStorage);
        }
        tokio::task::spawn_blocking(move || {
            let parent = path.parent().ok_or(GoogleConnectionError::InvalidStorage)?;
            fs::ensure_private_dir(parent).map_err(|_| GoogleConnectionError::StorageUnavailable)?;
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or(GoogleConnectionError::InvalidStorage)?;
            let owner = match lock::try_acquire(&parent.join(format!(".{name}.owner.lock")), LockMode::Exclusive)
                .map_err(|_| GoogleConnectionError::StorageUnavailable)?
            {
                LockAttempt::Acquired(lock) => lock,
                LockAttempt::Contended => return Err(GoogleConnectionError::StorageUnavailable),
            };
            Ok(Self {
                file: Some(FileStorage { path, _owner: owner }),
                max_bytes,
            })
        })
        .await
        .map_err(|_| GoogleConnectionError::StorageUnavailable)?
    }

    pub(super) fn write(&self, record: &Record) -> Result<(), GoogleConnectionError> {
        let epoch = unix_ms()?;
        let now = Instant::now();
        let tokens = record
            .tokens
            .as_ref()
            .map(|tokens| {
                Ok::<_, GoogleConnectionError>(TokenView {
                    access_token: tokens.access_token.expose(),
                    refresh_token: tokens.refresh_token(),
                    refresh_expires_at_ms: tokens
                        .refresh_expires_at
                        .map(|expiry| {
                            epoch
                                .checked_add(
                                    u64::try_from(expiry.saturating_duration_since(now).as_millis())
                                        .unwrap_or(u64::MAX),
                                )
                                .ok_or(GoogleConnectionError::InvalidRecord)
                        })
                        .transpose()?,
                    scopes: &tokens.scopes,
                })
            })
            .transpose()?;
        let view = RecordView {
            version: 1,
            client_id: &record.client_id,
            subject: record.subject.expose(),
            revision: record.revision,
            state: record.state,
            saved_at_ms: epoch,
            tokens,
        };
        let bytes = Zeroizing::new(serde_json::to_vec(&view).map_err(|_| GoogleConnectionError::InvalidRecord)?);
        if bytes.len() > self.max_bytes {
            return Err(GoogleConnectionError::InvalidRecord);
        }
        if let Some(file) = &self.file {
            fs::atomic_write_private(&file.path, &bytes).map_err(|_| GoogleConnectionError::StorageUnavailable)?;
        }
        Ok(())
    }

    pub(super) fn read(&self, client: &GoogleOAuthClient) -> Result<Option<Record>, GoogleConnectionError> {
        let Some(file) = &self.file else {
            return Ok(None);
        };
        let parent = ContainedDir::open_root(file.path.parent().ok_or(GoogleConnectionError::InvalidStorage)?)
            .map_err(|_| GoogleConnectionError::StorageUnavailable)?;
        parent
            .validate_private()
            .map_err(|_| GoogleConnectionError::StorageUnavailable)?;
        let handle = match parent
            .open_existing_private_append(file.path.file_name().ok_or(GoogleConnectionError::InvalidStorage)?)
        {
            Ok(handle) => handle,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(GoogleConnectionError::StorageUnavailable),
        };
        let mut bytes = Zeroizing::new(Vec::new());
        handle
            .take(self.max_bytes.saturating_add(1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| GoogleConnectionError::StorageUnavailable)?;
        if bytes.len() > self.max_bytes {
            return Err(GoogleConnectionError::InvalidRecord);
        }
        let stored: Stored = serde_json::from_slice(&bytes).map_err(|_| GoogleConnectionError::InvalidRecord)?;
        let epoch = unix_ms()?;
        let now = Instant::now();
        if stored.version != 1
            || stored.client_id != client.registration.client_id
            || stored.revision == 0
            || stored.revision > (1_u64 << 53) - 1
            || stored.saved_at_ms > epoch
            || !super::super::valid_token(stored.subject.expose())
            || stored.subject.expose().len() > 255
            || (stored.state == GoogleConnectionState::Disconnected && stored.tokens.is_some())
            || (stored.state == GoogleConnectionState::Connected && stored.tokens.is_none())
        {
            return Err(GoogleConnectionError::InvalidRecord);
        }
        let tokens = stored
            .tokens
            .map(|tokens| {
                if !super::super::valid_token(tokens.access_token.expose())
                    || tokens
                        .refresh_token
                        .as_ref()
                        .is_some_and(|token| !super::super::valid_token(token.expose()))
                    || tokens.scopes.is_empty()
                    || !tokens.scopes.is_subset(&client.approved_scopes)
                {
                    return Err(GoogleConnectionError::InvalidRecord);
                }
                let refresh_expires_at = tokens
                    .refresh_expires_at_ms
                    .map(|expiry| {
                        now.checked_add(Duration::from_millis(expiry.saturating_sub(epoch)))
                            .ok_or(GoogleConnectionError::InvalidRecord)
                    })
                    .transpose()?;
                // Never reuse an access token based on a restart's wall clock.
                Ok(Arc::new(OAuthTokens {
                    client_id: stored.client_id.clone(),
                    access_token: tokens.access_token,
                    refresh_token: tokens.refresh_token,
                    id_token: None,
                    expected_nonce: None,
                    expires_at: now,
                    refresh_expires_at,
                    scopes: tokens.scopes,
                }))
            })
            .transpose()?;
        Ok(Some(Record {
            client_id: Arc::from(stored.client_id),
            subject: Arc::new(stored.subject),
            revision: stored.revision,
            state: stored.state,
            tokens,
        }))
    }
}

fn unix_ms() -> Result<u64, GoogleConnectionError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|time| time.as_millis().try_into().ok())
        .ok_or(GoogleConnectionError::InvalidRecord)
}
#[derive(Serialize)]
struct RecordView<'a> {
    version: u8,
    client_id: &'a str,
    subject: &'a str,
    revision: u64,
    state: GoogleConnectionState,
    saved_at_ms: u64,
    tokens: Option<TokenView<'a>>,
}
#[derive(Serialize)]
struct TokenView<'a> {
    access_token: &'a str,
    refresh_token: Option<&'a str>,
    refresh_expires_at_ms: Option<u64>,
    scopes: &'a std::collections::BTreeSet<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    version: u8,
    client_id: String,
    subject: Secret,
    revision: u64,
    state: GoogleConnectionState,
    saved_at_ms: u64,
    tokens: Option<StoredTokens>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredTokens {
    access_token: Secret,
    refresh_token: Option<Secret>,
    refresh_expires_at_ms: Option<u64>,
    scopes: std::collections::BTreeSet<String>,
}
