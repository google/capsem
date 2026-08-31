use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tracing::{info, warn};

use crate::durable;
use crate::provider::credential_provider_from_str;
use crate::{is_broker_reference, CredentialProvider};

static CREDENTIAL_STORE: OnceLock<CredentialStore> = OnceLock::new();

/// Opaque credential storage boundary for the credential broker.
///
/// Runtime credential access goes through this object: substitution reads the
/// in-memory cache first, capture writes RAM first and then durable storage,
/// and startup/reload hydrates RAM from durable storage. Status callers only
/// see memory-owned state and cannot accidentally read durable credentials.
pub struct CredentialStore {
    cache: Mutex<HashMap<String, String>>,
    durable_lock: Mutex<()>,
    status: Mutex<CredentialStoreStatusState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CredentialStoreStatus {
    pub backend: String,
    pub ready: bool,
    pub status: &'static str,
    pub cached_count: usize,
    pub last_hydrated_count: usize,
    pub last_hydrated_unix_ms: Option<u64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CredentialStoreStatusState {
    ready: bool,
    last_hydrated_count: usize,
    last_hydrated_unix_ms: Option<u64>,
    last_error: Option<String>,
}

impl Default for CredentialStoreStatusState {
    fn default() -> Self {
        Self {
            ready: true,
            last_hydrated_count: 0,
            last_hydrated_unix_ms: None,
            last_error: None,
        }
    }
}

impl Default for CredentialStore {
    fn default() -> Self {
        Self {
            cache: Mutex::new(HashMap::new()),
            durable_lock: Mutex::new(()),
            status: Mutex::new(CredentialStoreStatusState::default()),
        }
    }
}

impl CredentialStore {
    pub fn global() -> &'static Self {
        CREDENTIAL_STORE.get_or_init(Self::default)
    }

    pub fn capture(
        &self,
        provider: CredentialProvider,
        credential_ref: &str,
        raw_value: &str,
    ) -> Result<bool, String> {
        if let Some(existing) = self.cache_get(provider, credential_ref)? {
            if existing == raw_value {
                return Ok(false);
            }
            return Err(format!(
                "credential reference collision for provider {} and ref {credential_ref}",
                provider.as_str()
            ));
        }
        self.cache_insert(provider, credential_ref, raw_value)?;
        let _durable_guard = self
            .durable_lock
            .lock()
            .map_err(|_| "credential durable store lock poisoned".to_string())?;
        if let Err(error) = durable::write(provider, credential_ref, raw_value) {
            self.mark_error(error.clone());
            warn!(
                provider = provider.as_str(),
                credential_ref,
                error = %error,
                "credential store: durable write failed; runtime cache remains available"
            );
        } else {
            self.clear_error();
            info!(
                provider = provider.as_str(),
                credential_ref, "credential store: credential captured into durable backend"
            );
        }
        Ok(true)
    }

    pub fn resolve(
        &self,
        provider: CredentialProvider,
        credential_ref: &str,
    ) -> Result<Option<String>, String> {
        if !is_broker_reference(credential_ref) {
            return Ok(None);
        }
        if let Some(raw_value) = self.cache_get(provider, credential_ref)? {
            return Ok(Some(raw_value));
        }
        let _durable_guard = self
            .durable_lock
            .lock()
            .map_err(|_| "credential durable store lock poisoned".to_string())?;
        match durable::read(provider, credential_ref) {
            Ok(raw_value) => {
                self.cache_insert(provider, credential_ref, &raw_value)?;
                self.clear_error();
                info!(
                    provider = provider.as_str(),
                    credential_ref, "credential store: hydrated credential on runtime miss"
                );
                Ok(Some(raw_value))
            }
            Err(error) => {
                self.mark_error(error.clone());
                Err(error)
            }
        }
    }

    pub fn replay_available_in_memory(
        &self,
        provider: CredentialProvider,
        credential_ref: &str,
    ) -> bool {
        self.cache_get(provider, credential_ref)
            .ok()
            .flatten()
            .is_some()
    }

    pub fn hydrate_from_durable_store(&self) -> Result<usize, String> {
        let _durable_guard = self
            .durable_lock
            .lock()
            .map_err(|_| "credential durable store lock poisoned".to_string())?;
        let entries = match durable::hydrate() {
            Ok(entries) => entries,
            Err(error) => {
                self.mark_degraded(error.clone());
                return Err(error);
            }
        };
        let count = entries.len();
        {
            let mut cache = self
                .cache
                .lock()
                .map_err(|_| "credential runtime cache lock poisoned".to_string())?;
            for (provider, credential_ref, raw_value) in entries {
                cache.insert(credential_store_key(provider, &credential_ref), raw_value);
            }
        }
        self.mark_hydrated(count);
        info!(
            count,
            "credential store: hydrated runtime cache from durable backend"
        );
        Ok(count)
    }

    pub fn status(&self) -> CredentialStoreStatus {
        let cached_count = self.cache.lock().map(|cache| cache.len()).unwrap_or(0);
        let state = self
            .status
            .lock()
            .map(|state| state.clone())
            .unwrap_or_else(|_| CredentialStoreStatusState {
                ready: false,
                last_hydrated_count: 0,
                last_hydrated_unix_ms: None,
                last_error: Some("credential store status lock poisoned".to_string()),
            });
        CredentialStoreStatus {
            backend: durable::backend_name().to_string(),
            ready: state.ready,
            status: if state.ready { "ready" } else { "degraded" },
            cached_count,
            last_hydrated_count: state.last_hydrated_count,
            last_hydrated_unix_ms: state.last_hydrated_unix_ms,
            last_error: state.last_error,
        }
    }

    #[doc(hidden)]
    pub fn clear_for_test(&self) {
        self.cache.lock().unwrap().clear();
        *self.status.lock().unwrap() = CredentialStoreStatusState::default();
    }

    fn cache_insert(
        &self,
        provider: CredentialProvider,
        credential_ref: &str,
        raw_value: &str,
    ) -> Result<(), String> {
        self.cache
            .lock()
            .map_err(|_| "credential runtime cache lock poisoned".to_string())?
            .insert(
                credential_store_key(provider, credential_ref),
                raw_value.to_string(),
            );
        Ok(())
    }

    fn cache_get(
        &self,
        provider: CredentialProvider,
        credential_ref: &str,
    ) -> Result<Option<String>, String> {
        Ok(self
            .cache
            .lock()
            .map_err(|_| "credential runtime cache lock poisoned".to_string())?
            .get(&credential_store_key(provider, credential_ref))
            .cloned())
    }

    fn mark_hydrated(&self, count: usize) {
        if let Ok(mut status) = self.status.lock() {
            status.ready = true;
            status.last_hydrated_count = count;
            status.last_hydrated_unix_ms = Some(now_unix_ms());
            status.last_error = None;
        }
    }

    fn mark_error(&self, error: String) {
        if let Ok(mut status) = self.status.lock() {
            status.last_error = Some(error);
        }
    }

    fn mark_degraded(&self, error: String) {
        if let Ok(mut status) = self.status.lock() {
            status.ready = false;
            status.last_error = Some(error);
        }
    }

    fn clear_error(&self) {
        if let Ok(mut status) = self.status.lock() {
            status.ready = true;
            status.last_error = None;
        }
    }
}

pub fn resolve_broker_reference_for_provider(
    provider: CredentialProvider,
    credential_ref: &str,
) -> Result<Option<String>, String> {
    CredentialStore::global().resolve(provider, credential_ref)
}

pub fn broker_reference_replay_available(provider: Option<&str>, credential_ref: &str) -> bool {
    let Some(provider) = provider.and_then(credential_provider_from_str) else {
        return CredentialProvider::all().iter().copied().any(|provider| {
            CredentialStore::global().replay_available_in_memory(provider, credential_ref)
        });
    };
    CredentialStore::global().replay_available_in_memory(provider, credential_ref)
}

pub fn hydrate_credential_runtime_cache_from_durable_store() -> Result<usize, String> {
    CredentialStore::global().hydrate_from_durable_store()
}

pub fn credential_store_status() -> CredentialStoreStatus {
    CredentialStore::global().status()
}

pub fn credential_store_account(provider: CredentialProvider, credential_ref: &str) -> String {
    format!("{}:{credential_ref}", provider.as_str())
}

fn credential_store_key(provider: CredentialProvider, credential_ref: &str) -> String {
    credential_store_account(provider, credential_ref)
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
