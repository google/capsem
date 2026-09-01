use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

pub use capsem_credentials::{
    broker_reference_replay_available, credential_store_account, credential_store_status,
    hydrate_credential_runtime_cache_from_durable_store, is_broker_reference, resolve_broker_reference_for_provider,
    CredentialProvider, CredentialStore, CredentialStoreStatus, STORE_PATH_ENV,
};
use capsem_logger::{credential_reference, DbWriter, SubstitutionEvent};
use tracing::warn;

use crate::net::ai_traffic::provider::ProviderKind;
use crate::net::policy_config::SecurityRuleSet;
use crate::security_engine::RuntimeSecurityEventType;

#[cfg(test)]
pub(crate) static TEST_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static LOGGED_CREDENTIAL_OBSERVATIONS: OnceLock<Mutex<HashSet<LoggedCredentialObservation>>> = OnceLock::new();

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
    pub store_account: String,
    pub newly_captured: bool,
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

pub fn broker_observed_credential(observation: &CredentialObservation) -> Result<BrokeredCredential, String> {
    let credential_ref = observation.credential_ref();
    let store_account = credential_store_account(observation.provider, &credential_ref);
    let newly_captured =
        CredentialStore::global().capture(observation.provider, &credential_ref, &observation.raw_value)?;
    Ok(BrokeredCredential {
        provider: observation.provider,
        credential_ref,
        store_account,
        newly_captured,
    })
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

pub fn detect_http_credential(domain: &str, header_name: &str, header_value: &[u8]) -> Option<CredentialObservation> {
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
            && (path.contains("/token") || path.contains("oauth") || path.contains("/credential/response")))
}

pub fn substitute_credential_value(provider: CredentialProvider, raw_value: &str) -> String {
    credential_reference(provider.as_str(), raw_value)
}

pub fn redact_observed_credentials_in_bytes(bytes: &[u8], observations: &[CredentialObservation]) -> Vec<u8> {
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
        let brokered = match tokio::task::spawn_blocking({
            let observation = observation.clone();
            move || broker_observed_credential(&observation)
        })
        .await
        {
            Ok(Ok(brokered)) => brokered,
            Ok(Err(error)) => {
                warn!(
                    provider = observation.provider.as_str(),
                    source = observation.source.as_str(),
                    error = %error,
                    "credential broker: failed to save observed credential"
                );
                crate::security_engine::emit_substitution_security_write_and_rules(
                    db,
                    rules,
                    observation.redacted_event("error"),
                )
                .await;
                continue;
            }
            Err(error) => {
                warn!(
                    provider = observation.provider.as_str(),
                    source = observation.source.as_str(),
                    error = %error,
                    "credential broker: save task failed"
                );
                crate::security_engine::emit_substitution_security_write_and_rules(
                    db,
                    rules,
                    observation.redacted_event("error"),
                )
                .await;
                continue;
            }
        };

        let first_logged_in_session = mark_credential_observation_logged(db, &observation, &brokered.credential_ref);
        if first_logged_in_session {
            crate::security_engine::emit_substitution_security_write_and_rules(
                db,
                rules,
                observation.redacted_event("captured"),
            )
            .await;
        }
        if first_logged_in_session {
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct LoggedCredentialObservation {
    db_path: PathBuf,
    provider: CredentialProvider,
    credential_ref: String,
    source: String,
    event_type: Option<String>,
}

fn mark_credential_observation_logged(
    db: &DbWriter,
    observation: &CredentialObservation,
    credential_ref: &str,
) -> bool {
    let key = LoggedCredentialObservation {
        db_path: db.path().to_path_buf(),
        provider: observation.provider,
        credential_ref: credential_ref.to_string(),
        source: observation.source.clone(),
        event_type: observation.event_type.clone(),
    };
    LOGGED_CREDENTIAL_OBSERVATIONS
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .map(|mut logged| logged.insert(key))
        .unwrap_or(true)
}

pub async fn log_brokered_injections(db: &DbWriter, rules: &SecurityRuleSet, injections: Vec<CredentialInjection>) {
    for injection in injections {
        crate::security_engine::emit_substitution_security_write_and_rules(
            db,
            rules,
            injection.redacted_event("injected"),
        )
        .await;
    }
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
        let Some(substitution) = substitute_brokered_header_value(text, provider_hint, &mut credential_ref)? else {
            continue;
        };
        *value = http::header::HeaderValue::from_str(&substitution)
            .map_err(|e| format!("stored credential is not valid header value: {e}"))?;
    }

    let query = match query {
        Some(q) => Some(substitute_brokered_query(q, provider_hint, &mut credential_ref)?),
        None => None,
    };

    Ok(BrokeredUpstreamCredentials { credential_ref, query })
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
    if let Some(reference) = bearer_value(trimmed).filter(|reference| is_broker_reference(reference)) {
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

fn resolve_broker_reference(provider_hint: Option<CredentialProvider>, credential_ref: &str) -> Result<String, String> {
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

fn credential_provider_for_request(domain: &str, ai_provider: Option<ProviderKind>) -> Option<CredentialProvider> {
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
                let hex =
                    std::str::from_utf8(&bytes[i + 1..i + 3]).map_err(|e| format!("invalid percent escape: {e}"))?;
                let byte = u8::from_str_radix(hex, 16).map_err(|e| format!("invalid percent escape %{hex}: {e}"))?;
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

fn provider_for_body_field(domain: &str, path: &str, field_name: &str, value: &str) -> Option<CredentialProvider> {
    provider_for_oauth_field(domain, path, field_name, value).or_else(|| provider_for_token(domain, field_name, value))
}

fn provider_for_oauth_field(domain: &str, path: &str, field_name: &str, value: &str) -> Option<CredentialProvider> {
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
    value.strip_prefix("Bearer ").or_else(|| value.strip_prefix("bearer "))
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

#[cfg(test)]
mod tests;
