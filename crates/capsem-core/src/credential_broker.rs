use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use capsem_logger::{credential_reference, DbWriter, SubstitutionEvent, CREDENTIAL_REF_PREFIX};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::net::ai_traffic::provider::ProviderKind;
use crate::net::policy_config::SecurityRuleSet;
use crate::security_engine::RuntimeSecurityEventType;

#[cfg(target_os = "macos")]
const KEYCHAIN_SERVICE: &str = "org.capsem.credentials";
#[cfg(target_os = "macos")]
const KEYCHAIN_VAULT_ACCOUNT: &str = "__capsem_credential_vault_v1";
pub(crate) const TEST_STORE_ENV: &str = "CAPSEM_CREDENTIAL_BROKER_TEST_STORE";
#[cfg(test)]
pub(crate) static TEST_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static TEST_STORE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static CREDENTIAL_STORE: OnceLock<CredentialStore> = OnceLock::new();
#[cfg(target_os = "macos")]
static KEYCHAIN_VAULT_CACHE: OnceLock<Mutex<Option<HashMap<String, String>>>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialProvider {
    Anthropic,
    Google,
    OpenAi,
    Github,
    Mcp,
}

impl CredentialProvider {
    pub fn all() -> &'static [Self] {
        &[
            Self::Anthropic,
            Self::Google,
            Self::OpenAi,
            Self::Github,
            Self::Mcp,
        ]
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::Google => "google",
            Self::OpenAi => "openai",
            Self::Github => "github",
            Self::Mcp => "mcp",
        }
    }
}

/// Opaque credential storage boundary for the credential broker.
///
/// All runtime credential access goes through this object: hot-path
/// substitution reads the in-memory cache first, capture writes RAM first and
/// then durable storage, and startup/reload hydrates RAM from durable storage.
/// UI/status callers must use the memory-only status helpers so they cannot
/// accidentally hammer Keychain.
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
    ) -> Result<(), String> {
        self.cache_insert(provider, credential_ref, raw_value)?;
        let _durable_guard = self
            .durable_lock
            .lock()
            .map_err(|_| "credential durable store lock poisoned".to_string())?;
        if let Err(error) = durable_store_write(provider, credential_ref, raw_value) {
            self.mark_error(error.clone());
            warn!(
                provider = provider.as_str(),
                credential_ref,
                error = %error,
                "credential store: durable write failed; runtime cache will continue serving active sessions"
            );
        } else {
            self.clear_error();
            info!(
                provider = provider.as_str(),
                credential_ref, "credential store: credential captured into durable backend"
            );
        }
        Ok(())
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
        match durable_store_read(provider, credential_ref) {
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
        let entries = match durable_store_hydrate() {
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
            backend: credential_store_backend().to_string(),
            ready: state.ready,
            status: if state.ready { "ready" } else { "degraded" },
            cached_count,
            last_hydrated_count: state.last_hydrated_count,
            last_hydrated_unix_ms: state.last_hydrated_unix_ms,
            last_error: state.last_error,
        }
    }

    #[cfg(test)]
    fn clear_for_test(&self) {
        self.cache.lock().unwrap().clear();
        *self.status.lock().unwrap() = CredentialStoreStatusState::default();
    }

    fn cache_insert(
        &self,
        provider: CredentialProvider,
        credential_ref: &str,
        raw_value: &str,
    ) -> Result<(), String> {
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| "credential runtime cache lock poisoned".to_string())?;
        cache.insert(
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
        let cache = self
            .cache
            .lock()
            .map_err(|_| "credential runtime cache lock poisoned".to_string())?;
        Ok(cache
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialObservation {
    pub provider: CredentialProvider,
    pub raw_value: String,
    pub source: String,
    pub event_type: Option<String>,
    pub trace_id: Option<String>,
    pub context_json: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialInjection {
    pub provider: Option<CredentialProvider>,
    pub credential_ref: String,
    pub source: String,
    pub event_type: Option<String>,
    pub trace_id: Option<String>,
    pub context_json: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokeredCredential {
    pub provider: CredentialProvider,
    pub credential_ref: String,
    pub keychain_account: String,
}

impl CredentialObservation {
    pub fn credential_ref(&self) -> String {
        credential_reference(self.provider.as_str(), &self.raw_value)
    }

    pub fn redacted_event(&self, outcome: &str) -> SubstitutionEvent {
        SubstitutionEvent {
            event_id: None,
            timestamp: std::time::SystemTime::now(),
            material_class: "credential".to_string(),
            source: self.source.clone(),
            event_type: self.event_type.clone(),
            algorithm: "blake3".to_string(),
            substitution_ref: self.credential_ref(),
            outcome: outcome.to_string(),
            provider: Some(self.provider.as_str().to_string()),
            confidence: None,
            trace_id: self.trace_id.clone(),
            context_json: self.context_json.clone(),
        }
    }
}

impl CredentialInjection {
    pub fn redacted_event(&self, outcome: &str) -> SubstitutionEvent {
        SubstitutionEvent {
            event_id: None,
            timestamp: std::time::SystemTime::now(),
            material_class: "credential".to_string(),
            source: self.source.clone(),
            event_type: self.event_type.clone(),
            algorithm: "blake3".to_string(),
            substitution_ref: self.credential_ref.clone(),
            outcome: outcome.to_string(),
            provider: self.provider.map(|provider| provider.as_str().to_string()),
            confidence: None,
            trace_id: self.trace_id.clone(),
            context_json: self.context_json.clone(),
        }
    }
}

pub fn broker_observed_credential(
    observation: &CredentialObservation,
) -> Result<BrokeredCredential, String> {
    let credential_ref = observation.credential_ref();
    let keychain_account = keychain_account(observation.provider, &credential_ref);
    CredentialStore::global().capture(
        observation.provider,
        &credential_ref,
        &observation.raw_value,
    )?;
    Ok(BrokeredCredential {
        provider: observation.provider,
        credential_ref,
        keychain_account,
    })
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

#[cfg(target_os = "macos")]
pub const fn credential_broker_keychain_service() -> &'static str {
    KEYCHAIN_SERVICE
}

#[cfg(not(target_os = "macos"))]
pub const fn credential_broker_keychain_service() -> &'static str {
    "org.capsem.credentials"
}

fn credential_provider_from_str(provider: &str) -> Option<CredentialProvider> {
    match provider {
        "anthropic" => Some(CredentialProvider::Anthropic),
        "google" => Some(CredentialProvider::Google),
        "openai" => Some(CredentialProvider::OpenAi),
        "github" => Some(CredentialProvider::Github),
        "mcp" => Some(CredentialProvider::Mcp),
        _ => None,
    }
}

pub fn keychain_account(provider: CredentialProvider, credential_ref: &str) -> String {
    format!("{}:{credential_ref}", provider.as_str())
}

pub fn parse_env_credentials(source_path: &str, content: &str) -> Vec<CredentialObservation> {
    content
        .lines()
        .filter_map(parse_env_assignment)
        .filter_map(|(name, raw_value)| {
            provider_for_env_name(name).map(|provider| CredentialObservation {
                provider,
                raw_value: raw_value.to_string(),
                source: format!("{source_path}:{name}"),
                event_type: Some(RuntimeSecurityEventType::FileEvent.as_str().to_string()),
                trace_id: None,
                context_json: Some(format!(
                    r#"{{"path":"{}","env":"{}"}}"#,
                    json_escape(source_path),
                    json_escape(name)
                )),
            })
        })
        .collect()
}

pub fn detect_http_credential(
    domain: &str,
    header_name: &str,
    header_value: &[u8],
) -> Option<CredentialObservation> {
    detect_http_credential_with_provider(domain, None, header_name, header_value)
}

pub fn detect_http_credential_with_provider(
    domain: &str,
    ai_provider: Option<ProviderKind>,
    header_name: &str,
    header_value: &[u8],
) -> Option<CredentialObservation> {
    let value = std::str::from_utf8(header_value).ok()?.trim();
    if value.is_empty() {
        return None;
    }
    if header_broker_reference(value).is_some() {
        return None;
    }
    let raw = bearer_value(value).unwrap_or(value).trim();
    let provider = provider_for_token(domain, header_name, raw)
        .or_else(|| provider_for_header_hint(domain, ai_provider, header_name, raw))?;
    Some(CredentialObservation {
        provider,
        raw_value: raw.to_string(),
        source: format!("http.header.{}", header_name.to_ascii_lowercase()),
        event_type: Some("http.request".to_string()),
        trace_id: None,
        context_json: Some(format!(
            r#"{{"domain":"{}","header":"{}"}}"#,
            json_escape(domain),
            json_escape(header_name)
        )),
    })
}

fn provider_for_header_hint(
    domain: &str,
    ai_provider: Option<ProviderKind>,
    header_name: &str,
    raw: &str,
) -> Option<CredentialProvider> {
    if raw.is_empty() {
        return None;
    }
    let header = header_name.to_ascii_lowercase();
    if header == "x-goog-api-key" {
        return Some(CredentialProvider::Google);
    }
    if matches!(ai_provider, Some(ProviderKind::Unknown)) && header == "authorization" {
        return Some(CredentialProvider::OpenAi);
    }
    if matches!(ai_provider, Some(ProviderKind::Unknown)) && header == "x-api-key" {
        return Some(CredentialProvider::Anthropic);
    }
    let credential_header = header == "authorization"
        || header == "x-api-key"
        || header == "x-goog-api-key"
        || header == "api-key"
        || header == "apikey";
    credential_header
        .then(|| credential_provider_for_request(domain, ai_provider))
        .flatten()
}

pub fn detect_http_body_credentials(
    domain: &str,
    path: &str,
    direction: &str,
    body: &[u8],
) -> Vec<CredentialObservation> {
    let Ok(text) = std::str::from_utf8(body) else {
        return Vec::new();
    };

    let mut found = Vec::new();
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(text) {
        collect_json_credentials(domain, path, direction, "$", &json, &mut found);
        return found;
    }

    collect_form_credentials(domain, path, direction, text, &mut found);
    found
}

pub fn detect_brokered_http_references(
    domain: &str,
    ai_provider: Option<ProviderKind>,
    headers: &http::HeaderMap,
    query: Option<&str>,
    trace_id: Option<String>,
) -> Vec<CredentialInjection> {
    let mut found = Vec::new();
    let provider_hint = credential_provider_for_request(domain, ai_provider);
    for (name, value) in headers.iter() {
        let Some(reference) = value
            .to_str()
            .ok()
            .and_then(|value| header_broker_reference(value).map(str::to_string))
        else {
            continue;
        };
        found.push(CredentialInjection {
            provider: provider_hint.or_else(|| provider_for_stored_reference(&reference)),
            credential_ref: reference,
            source: format!("http.header.{}", name.as_str().to_ascii_lowercase()),
            event_type: Some("http.request".to_string()),
            trace_id: trace_id.clone(),
            context_json: Some(format!(
                r#"{{"domain":"{}","header":"{}"}}"#,
                json_escape(domain),
                json_escape(name.as_str())
            )),
        });
    }
    if let Some(query) = query {
        collect_query_brokered_references(domain, provider_hint, query, trace_id, &mut found);
    }
    found
}

pub fn is_http_body_credential_candidate(domain: &str, path: &str) -> bool {
    (domain.ends_with("googleapis.com") && (path.contains("/token") || path.contains("oauth")))
        || (domain.ends_with("github.com") && path.contains("oauth"))
        || (is_local_oauth_fixture_domain(domain)
            && (path.contains("/token")
                || path.contains("oauth")
                || path.contains("/credential/response")))
}

pub fn substitute_credential_value(provider: CredentialProvider, raw_value: &str) -> String {
    credential_reference(provider.as_str(), raw_value)
}

pub fn redact_observed_credentials_in_bytes(
    bytes: &[u8],
    observations: &[CredentialObservation],
) -> Vec<u8> {
    if observations.is_empty() {
        return bytes.to_vec();
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        return bytes.to_vec();
    };
    let mut redacted = text.to_string();
    for observation in observations {
        redacted = redacted.replace(&observation.raw_value, &observation.credential_ref());
        let encoded = percent_encode_query_value(&observation.raw_value);
        if encoded != observation.raw_value {
            redacted = redacted.replace(&encoded, &observation.credential_ref());
        }
    }
    redacted.into_bytes()
}

pub async fn broker_and_log_observations(
    db: &DbWriter,
    rules: &SecurityRuleSet,
    observations: Vec<CredentialObservation>,
) -> Option<String> {
    let mut first_ref = None;
    let mut seen = HashSet::new();
    for observation in observations {
        let reference = observation.credential_ref();
        let key = (
            observation.provider,
            reference.clone(),
            observation.source.clone(),
            observation.event_type.clone(),
            observation.trace_id.clone(),
            observation.context_json.clone(),
        );
        if !seen.insert(key) {
            continue;
        }
        if first_ref.is_none() {
            first_ref = Some(reference);
        }
        let save_outcome = match tokio::task::spawn_blocking({
            let observation = observation.clone();
            move || broker_observed_credential(&observation)
        })
        .await
        {
            Ok(Ok(_)) => "captured",
            Ok(Err(error)) => {
                warn!(
                    provider = observation.provider.as_str(),
                    source = observation.source.as_str(),
                    error = %error,
                    "credential broker: failed to save observed credential"
                );
                "error"
            }
            Err(error) => {
                warn!(
                    provider = observation.provider.as_str(),
                    source = observation.source.as_str(),
                    error = %error,
                    "credential broker: save task failed"
                );
                "error"
            }
        };
        crate::security_engine::emit_substitution_security_write_and_rules(
            db,
            rules,
            observation.redacted_event(save_outcome),
        )
        .await;
        if save_outcome == "captured" {
            crate::security_engine::emit_substitution_security_write_and_rules(
                db,
                rules,
                observation.redacted_event("brokered"),
            )
            .await;
        }
    }
    first_ref
}

pub async fn log_brokered_injections(
    db: &DbWriter,
    rules: &SecurityRuleSet,
    injections: Vec<CredentialInjection>,
) {
    for injection in injections {
        crate::security_engine::emit_substitution_security_write_and_rules(
            db,
            rules,
            injection.redacted_event("injected"),
        )
        .await;
    }
}

pub fn is_broker_reference(value: &str) -> bool {
    value.starts_with(CREDENTIAL_REF_PREFIX) && capsem_logger::is_credential_reference(value)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokeredUpstreamCredentials {
    pub credential_ref: Option<String>,
    pub query: Option<String>,
}

pub fn substitute_brokered_upstream_credentials(
    domain: &str,
    ai_provider: Option<ProviderKind>,
    headers: &mut http::HeaderMap,
    query: Option<&str>,
) -> Result<BrokeredUpstreamCredentials, String> {
    let provider_hint = credential_provider_for_request(domain, ai_provider);
    let mut credential_ref = None;

    for value in headers.values_mut() {
        let text = value
            .to_str()
            .map_err(|e| format!("broker reference header is not UTF-8: {e}"))?;
        let Some(substitution) =
            substitute_brokered_header_value(text, provider_hint, &mut credential_ref)?
        else {
            continue;
        };
        *value = http::header::HeaderValue::from_str(&substitution)
            .map_err(|e| format!("stored credential is not valid header value: {e}"))?;
    }

    let query = match query {
        Some(q) => Some(substitute_brokered_query(
            q,
            provider_hint,
            &mut credential_ref,
        )?),
        None => None,
    };

    Ok(BrokeredUpstreamCredentials {
        credential_ref,
        query,
    })
}

fn substitute_brokered_header_value(
    value: &str,
    provider_hint: Option<CredentialProvider>,
    credential_ref: &mut Option<String>,
) -> Result<Option<String>, String> {
    let trimmed = value.trim();
    if is_broker_reference(trimmed) {
        let raw = resolve_broker_reference(provider_hint, trimmed)?;
        if credential_ref.is_none() {
            *credential_ref = Some(trimmed.to_string());
        }
        return Ok(Some(raw));
    }
    if let Some(reference) =
        bearer_value(trimmed).filter(|reference| is_broker_reference(reference))
    {
        let raw = resolve_broker_reference(provider_hint, reference)?;
        if credential_ref.is_none() {
            *credential_ref = Some(reference.to_string());
        }
        let prefix = if trimmed.starts_with("bearer ") {
            "bearer "
        } else {
            "Bearer "
        };
        return Ok(Some(format!("{prefix}{raw}")));
    }
    Ok(None)
}

fn substitute_brokered_query(
    query: &str,
    provider_hint: Option<CredentialProvider>,
    credential_ref: &mut Option<String>,
) -> Result<String, String> {
    let mut changed = false;
    let mut parts = Vec::new();
    for part in query.split('&') {
        let Some((name, value)) = part.split_once('=') else {
            parts.push(part.to_string());
            continue;
        };
        let decoded = percent_decode(value)?;
        if is_broker_reference(&decoded) {
            let raw = resolve_broker_reference(provider_hint, &decoded)?;
            if credential_ref.is_none() {
                *credential_ref = Some(decoded);
            }
            parts.push(format!("{name}={}", percent_encode_query_value(&raw)));
            changed = true;
        } else {
            parts.push(part.to_string());
        }
    }

    if changed {
        Ok(parts.join("&"))
    } else {
        Ok(query.to_string())
    }
}

fn resolve_broker_reference(
    provider_hint: Option<CredentialProvider>,
    credential_ref: &str,
) -> Result<String, String> {
    if let Some(provider) = provider_hint {
        if let Ok(Some(raw)) = resolve_broker_reference_for_provider(provider, credential_ref) {
            return Ok(raw);
        }
    }

    for provider in CredentialProvider::all()
        .iter()
        .copied()
        .filter(|provider| Some(*provider) != provider_hint)
    {
        if let Ok(Some(raw)) = resolve_broker_reference_for_provider(provider, credential_ref) {
            return Ok(raw);
        }
    }

    Err("credential broker reference could not be resolved".to_string())
}

fn provider_for_stored_reference(credential_ref: &str) -> Option<CredentialProvider> {
    CredentialProvider::all().iter().copied().find(|provider| {
        resolve_broker_reference_for_provider(*provider, credential_ref)
            .ok()
            .flatten()
            .is_some()
    })
}

fn collect_query_brokered_references(
    domain: &str,
    provider_hint: Option<CredentialProvider>,
    query: &str,
    trace_id: Option<String>,
    out: &mut Vec<CredentialInjection>,
) {
    for part in query.split('&') {
        let Some((name, value)) = part.split_once('=') else {
            continue;
        };
        let Ok(decoded) = percent_decode(value) else {
            continue;
        };
        if !is_broker_reference(&decoded) {
            continue;
        }
        out.push(CredentialInjection {
            provider: provider_hint.or_else(|| provider_for_stored_reference(&decoded)),
            credential_ref: decoded,
            source: format!("http.query.{name}"),
            event_type: Some("http.request".to_string()),
            trace_id: trace_id.clone(),
            context_json: Some(format!(
                r#"{{"domain":"{}","query_key":"{}"}}"#,
                json_escape(domain),
                json_escape(name)
            )),
        });
    }
}

fn credential_provider_for_request(
    domain: &str,
    ai_provider: Option<ProviderKind>,
) -> Option<CredentialProvider> {
    match ai_provider {
        Some(ProviderKind::Anthropic) => Some(CredentialProvider::Anthropic),
        Some(ProviderKind::Google) => Some(CredentialProvider::Google),
        Some(ProviderKind::OpenAi) => Some(CredentialProvider::OpenAi),
        Some(ProviderKind::Ollama) => Some(CredentialProvider::OpenAi),
        Some(ProviderKind::Unknown) => None,
        None if domain.ends_with("anthropic.com") || domain.ends_with("claude.com") => {
            Some(CredentialProvider::Anthropic)
        }
        None if domain.ends_with("googleapis.com") => Some(CredentialProvider::Google),
        None if domain.ends_with("openai.com") => Some(CredentialProvider::OpenAi),
        None if domain.ends_with("github.com") => Some(CredentialProvider::Github),
        None => None,
    }
}

fn percent_decode(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3])
                    .map_err(|e| format!("invalid percent escape: {e}"))?;
                let byte = u8::from_str_radix(hex, 16)
                    .map_err(|e| format!("invalid percent escape %{hex}: {e}"))?;
                out.push(byte);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).map_err(|e| format!("query value is not UTF-8: {e}"))
}

fn percent_encode_query_value(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn parse_env_assignment(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let trimmed = trimmed.strip_prefix("export ").unwrap_or(trimmed);
    let (name, value) = trimmed.split_once('=')?;
    let name = name.trim();
    let value = unquote(value.trim());
    if name.is_empty() || value.is_empty() {
        return None;
    }
    Some((name, value))
}

fn provider_for_env_name(name: &str) -> Option<CredentialProvider> {
    match name {
        "ANTHROPIC_API_KEY" => Some(CredentialProvider::Anthropic),
        "OPENAI_API_KEY" => Some(CredentialProvider::OpenAi),
        "GEMINI_API_KEY" | "GOOGLE_API_KEY" => Some(CredentialProvider::Google),
        "GITHUB_TOKEN" | "GH_TOKEN" => Some(CredentialProvider::Github),
        _ => None,
    }
}

fn provider_for_token(domain: &str, header_name: &str, token: &str) -> Option<CredentialProvider> {
    let header = header_name.to_ascii_lowercase();
    if token.starts_with("sk-ant-") {
        return Some(CredentialProvider::Anthropic);
    }
    if token.starts_with("sk-") {
        return Some(CredentialProvider::OpenAi);
    }
    if token.starts_with("AIza") {
        return Some(CredentialProvider::Google);
    }
    if token.starts_with("ghp_")
        || token.starts_with("github_pat_")
        || token.starts_with("gho_")
        || token.starts_with("ghu_")
        || token.starts_with("ghs_")
        || token.starts_with("ghr_")
    {
        return Some(CredentialProvider::Github);
    }
    if domain.ends_with("github.com")
        && (header == "authorization"
            || header == "access_token"
            || header == "refresh_token"
            || header.ends_with("_token")
            || header.ends_with("token"))
    {
        return Some(CredentialProvider::Github);
    }
    None
}

fn collect_json_credentials(
    domain: &str,
    path: &str,
    direction: &str,
    json_path: &str,
    value: &serde_json::Value,
    out: &mut Vec<CredentialObservation>,
) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                let child_path = format!("{json_path}.{key}");
                if let Some(raw) = child.as_str() {
                    if let Some(provider) = provider_for_body_field(domain, path, key, raw.trim()) {
                        out.push(CredentialObservation {
                            provider,
                            raw_value: raw.trim().to_string(),
                            source: format!("http.body.{direction}.{child_path}"),
                            event_type: Some(format!("http.{direction}")),
                            trace_id: None,
                            context_json: Some(format!(
                                r#"{{"domain":"{}","path":"{}","json_path":"{}","direction":"{}"}}"#,
                                json_escape(domain),
                                json_escape(path),
                                json_escape(&child_path),
                                json_escape(direction)
                            )),
                        });
                    }
                }
                collect_json_credentials(domain, path, direction, &child_path, child, out);
            }
        }
        serde_json::Value::Array(items) => {
            for (idx, child) in items.iter().enumerate() {
                let child_path = format!("{json_path}[{idx}]");
                collect_json_credentials(domain, path, direction, &child_path, child, out);
            }
        }
        _ => {}
    }
}

fn collect_form_credentials(
    domain: &str,
    path: &str,
    direction: &str,
    text: &str,
    out: &mut Vec<CredentialObservation>,
) {
    if !text.contains('=') {
        return;
    }
    for part in text.split('&') {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        let Ok(raw) = percent_decode(value) else {
            continue;
        };
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        if let Some(provider) = provider_for_body_field(domain, path, key, raw) {
            out.push(CredentialObservation {
                provider,
                raw_value: raw.to_string(),
                source: format!("http.body.{direction}.form.{key}"),
                event_type: Some(format!("http.{direction}")),
                trace_id: None,
                context_json: Some(format!(
                    r#"{{"domain":"{}","path":"{}","form_key":"{}","direction":"{}"}}"#,
                    json_escape(domain),
                    json_escape(path),
                    json_escape(key),
                    json_escape(direction)
                )),
            });
        }
    }
}

fn provider_for_body_field(
    domain: &str,
    path: &str,
    field_name: &str,
    value: &str,
) -> Option<CredentialProvider> {
    provider_for_oauth_field(domain, path, field_name, value)
        .or_else(|| provider_for_token(domain, field_name, value))
}

fn provider_for_oauth_field(
    domain: &str,
    path: &str,
    field_name: &str,
    value: &str,
) -> Option<CredentialProvider> {
    if value.trim().is_empty() {
        return None;
    }
    let field = field_name.to_ascii_lowercase();
    if !matches!(
        field.as_str(),
        "access_token" | "refresh_token" | "id_token" | "code" | "device_code" | "client_secret"
    ) {
        return None;
    }
    if domain.ends_with("googleapis.com") && is_http_body_credential_candidate(domain, path) {
        return Some(CredentialProvider::Google);
    }
    if domain.ends_with("github.com") && is_http_body_credential_candidate(domain, path) {
        return Some(CredentialProvider::Github);
    }
    if is_local_oauth_fixture_domain(domain) && is_http_body_credential_candidate(domain, path) {
        return Some(CredentialProvider::Google);
    }
    None
}

fn is_local_oauth_fixture_domain(domain: &str) -> bool {
    matches!(domain, "127.0.0.1" | "localhost" | "::1")
}

fn bearer_value(value: &str) -> Option<&str> {
    value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))
}

fn header_broker_reference(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if is_broker_reference(trimmed) {
        return Some(trimmed);
    }
    bearer_value(trimmed).filter(|reference| is_broker_reference(reference))
}

fn unquote(value: &str) -> &str {
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        if (bytes[0] == b'"' && bytes[value.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[value.len() - 1] == b'\'')
        {
            return &value[1..value.len() - 1];
        }
    }
    value
}

fn json_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn credential_store_key(provider: CredentialProvider, credential_ref: &str) -> String {
    keychain_account(provider, credential_ref)
}

fn credential_store_backend() -> &'static str {
    if test_store_path().is_some() {
        return "test_disk";
    }
    credential_store_backend_native()
}

#[cfg(target_os = "macos")]
fn credential_store_backend_native() -> &'static str {
    "keychain"
}

#[cfg(not(target_os = "macos"))]
fn credential_store_backend_native() -> &'static str {
    "disk"
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn durable_store_write(
    provider: CredentialProvider,
    credential_ref: &str,
    raw_value: &str,
) -> Result<(), String> {
    if let Some(path) = test_store_path() {
        return disk_store_write(&path, provider, credential_ref, raw_value);
    }
    durable_store_write_native(provider, credential_ref, raw_value)
}

fn durable_store_read(
    provider: CredentialProvider,
    credential_ref: &str,
) -> Result<String, String> {
    if let Some(path) = test_store_path() {
        return disk_store_read(&path, provider, credential_ref);
    }
    durable_store_read_native(provider, credential_ref)
}

fn durable_store_hydrate() -> Result<Vec<(CredentialProvider, String, String)>, String> {
    if let Some(path) = test_store_path() {
        return disk_store_hydrate(&path);
    }
    durable_store_hydrate_native()
}

fn test_store_path() -> Option<PathBuf> {
    std::env::var_os(TEST_STORE_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

#[cfg(not(target_os = "macos"))]
fn disk_credential_store_path() -> PathBuf {
    crate::paths::capsem_home()
        .join("credentials")
        .join("credential-store.json")
}

fn disk_store_write(
    path: &PathBuf,
    provider: CredentialProvider,
    credential_ref: &str,
    raw_value: &str,
) -> Result<(), String> {
    let _guard = test_store_lock()
        .lock()
        .map_err(|_| "credential disk store lock poisoned".to_string())?;
    let mut map = disk_store_load(path)?;
    map.insert(
        keychain_account(provider, credential_ref),
        raw_value.to_string(),
    );
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create credential test store dir: {e}"))?;
    }
    let json = serde_json::to_string_pretty(&map)
        .map_err(|e| format!("serialize credential disk store: {e}"))?;
    std::fs::write(path, json).map_err(|e| format!("write credential disk store: {e}"))?;
    restrict_secret_file(path)?;
    Ok(())
}

fn disk_store_read(
    path: &PathBuf,
    provider: CredentialProvider,
    credential_ref: &str,
) -> Result<String, String> {
    let _guard = test_store_lock()
        .lock()
        .map_err(|_| "credential disk store lock poisoned".to_string())?;
    let map = disk_store_load(path)?;
    let account = keychain_account(provider, credential_ref);
    map.get(&account)
        .cloned()
        .ok_or_else(|| format!("credential reference not found in disk store: {account}"))
}

fn disk_store_hydrate(path: &PathBuf) -> Result<Vec<(CredentialProvider, String, String)>, String> {
    let _guard = test_store_lock()
        .lock()
        .map_err(|_| "credential disk store lock poisoned".to_string())?;
    let map = disk_store_load(path)?;
    let mut entries = Vec::new();
    for (account, raw_value) in map {
        let Some((provider, credential_ref)) = parse_credential_store_account(&account) else {
            warn!(account, "credential store: ignoring malformed disk account");
            continue;
        };
        entries.push((provider, credential_ref.to_string(), raw_value));
    }
    Ok(entries)
}

fn test_store_lock() -> &'static Mutex<()> {
    TEST_STORE_LOCK.get_or_init(|| Mutex::new(()))
}

fn disk_store_load(path: &PathBuf) -> Result<HashMap<String, String>, String> {
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("read credential disk store: {e}"))?;
    if text.trim().is_empty() {
        return Ok(HashMap::new());
    }
    serde_json::from_str(&text).map_err(|e| format!("parse credential disk store: {e}"))
}

#[cfg(target_os = "macos")]
fn durable_store_write_native(
    provider: CredentialProvider,
    credential_ref: &str,
    raw_value: &str,
) -> Result<(), String> {
    let mut vault = keychain_read_vault().unwrap_or_else(|error| {
        warn!(error = %error, "credential store: rebuilding empty keychain vault");
        HashMap::new()
    });
    vault.insert(
        keychain_account(provider, credential_ref),
        raw_value.to_string(),
    );
    keychain_write_vault(&vault)
}

#[cfg(not(target_os = "macos"))]
fn durable_store_write_native(
    provider: CredentialProvider,
    credential_ref: &str,
    raw_value: &str,
) -> Result<(), String> {
    disk_store_write(
        &disk_credential_store_path(),
        provider,
        credential_ref,
        raw_value,
    )
}

#[cfg(target_os = "macos")]
fn durable_store_read_native(
    provider: CredentialProvider,
    credential_ref: &str,
) -> Result<String, String> {
    let vault = keychain_read_vault()?;
    let account = keychain_account(provider, credential_ref);
    vault
        .get(&account)
        .cloned()
        .ok_or_else(|| format!("credential reference not found in keychain vault: {account}"))
}

#[cfg(not(target_os = "macos"))]
fn durable_store_read_native(
    provider: CredentialProvider,
    credential_ref: &str,
) -> Result<String, String> {
    disk_store_read(&disk_credential_store_path(), provider, credential_ref)
}

#[cfg(target_os = "macos")]
fn durable_store_hydrate_native() -> Result<Vec<(CredentialProvider, String, String)>, String> {
    let vault = keychain_read_vault()?;
    let mut hydrated = Vec::new();
    for (account, raw_value) in vault {
        let Some((provider, credential_ref)) = parse_credential_store_account(&account) else {
            warn!(
                account,
                "credential store: ignoring malformed keychain vault account"
            );
            continue;
        };
        hydrated.push((provider, credential_ref.to_string(), raw_value));
    }
    Ok(hydrated)
}

#[cfg(not(target_os = "macos"))]
fn durable_store_hydrate_native() -> Result<Vec<(CredentialProvider, String, String)>, String> {
    disk_store_hydrate(&disk_credential_store_path())
}

fn parse_credential_store_account(account: &str) -> Option<(CredentialProvider, &str)> {
    let (provider, credential_ref) = account.split_once(':')?;
    let provider = credential_provider_from_str(provider)?;
    Some((provider, credential_ref))
}

#[cfg(unix)]
fn restrict_secret_file(path: &PathBuf) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("restrict credential disk store permissions: {e}"))
}

#[cfg(not(unix))]
fn restrict_secret_file(_path: &PathBuf) -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "macos")]
fn keychain_read_vault() -> Result<HashMap<String, String>, String> {
    let cache = KEYCHAIN_VAULT_CACHE.get_or_init(|| Mutex::new(None));
    let mut guard = cache
        .lock()
        .map_err(|_| "credential keychain vault cache lock poisoned".to_string())?;
    if let Some(vault) = guard.as_ref() {
        return Ok(vault.clone());
    }
    match keychain_read_account(KEYCHAIN_VAULT_ACCOUNT) {
        Ok(raw) => {
            let vault: HashMap<String, String> =
                serde_json::from_str(&raw).map_err(|e| format!("parse keychain vault: {e}"))?;
            *guard = Some(vault.clone());
            Ok(vault)
        }
        Err(_) => {
            let vault = HashMap::new();
            *guard = Some(vault.clone());
            Ok(vault)
        }
    }
}

#[cfg(target_os = "macos")]
fn keychain_write_vault(vault: &HashMap<String, String>) -> Result<(), String> {
    let raw = serde_json::to_string(vault).map_err(|e| format!("serialize keychain vault: {e}"))?;
    keychain_write_account(KEYCHAIN_VAULT_ACCOUNT, &raw)?;
    let cache = KEYCHAIN_VAULT_CACHE.get_or_init(|| Mutex::new(None));
    let mut guard = cache
        .lock()
        .map_err(|_| "credential keychain vault cache lock poisoned".to_string())?;
    *guard = Some(vault.clone());
    Ok(())
}

#[cfg(target_os = "macos")]
fn keychain_read_account(account: &str) -> Result<String, String> {
    use security_framework::os::macos::keychain::SecKeychain;

    let keychain = SecKeychain::default().map_err(|e| format!("open default keychain: {e}"))?;
    let (password, _) = keychain
        .find_generic_password(KEYCHAIN_SERVICE, account)
        .map_err(|e| format!("read credential from keychain: {e}"))?;
    String::from_utf8(password.as_ref().to_vec())
        .map_err(|e| format!("credential in keychain is not UTF-8: {e}"))
}

#[cfg(target_os = "macos")]
fn keychain_write_account(account: &str, raw_value: &str) -> Result<(), String> {
    use security_framework::os::macos::keychain::SecKeychain;

    let keychain = SecKeychain::default().map_err(|e| format!("open default keychain: {e}"))?;
    keychain
        .set_generic_password(KEYCHAIN_SERVICE, account, raw_value.as_bytes())
        .map_err(|e| format!("write credential to keychain: {e}"))
}

#[cfg(test)]
mod tests;
