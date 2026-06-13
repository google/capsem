use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use capsem_logger::{credential_reference, DbWriter, SubstitutionEvent, CREDENTIAL_REF_PREFIX};
use tracing::warn;

use crate::net::ai_traffic::provider::ProviderKind;
use crate::net::policy_config::SecurityRuleSet;
use crate::security_engine::RuntimeSecurityEventType;

#[cfg(target_os = "macos")]
const KEYCHAIN_SERVICE: &str = "com.capsem.credentials";
pub(crate) const TEST_STORE_ENV: &str = "CAPSEM_CREDENTIAL_BROKER_TEST_STORE";
#[cfg(test)]
pub(crate) static TEST_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static TEST_STORE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq)]
pub struct CredentialObservation {
    pub provider: CredentialProvider,
    pub raw_value: String,
    pub source: String,
    pub event_type: Option<String>,
    pub confidence: f64,
    pub trace_id: Option<String>,
    pub context_json: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CredentialInjection {
    pub provider: Option<CredentialProvider>,
    pub credential_ref: String,
    pub source: String,
    pub event_type: Option<String>,
    pub confidence: f64,
    pub trace_id: Option<String>,
    pub context_json: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
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
            confidence: Some(self.confidence),
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
            confidence: Some(self.confidence),
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
    store_credential_secret(
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
    if !is_broker_reference(credential_ref) {
        return Ok(None);
    }
    load_credential_secret(provider, credential_ref).map(Some)
}

pub fn broker_reference_replay_available(provider: Option<&str>, credential_ref: &str) -> bool {
    let Some(provider) = provider.and_then(credential_provider_from_str) else {
        return CredentialProvider::all().iter().copied().any(|provider| {
            resolve_broker_reference_for_provider(provider, credential_ref)
                .ok()
                .flatten()
                .is_some()
        });
    };
    resolve_broker_reference_for_provider(provider, credential_ref)
        .ok()
        .flatten()
        .is_some()
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
                confidence: 1.0,
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
        confidence: 1.0,
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
            confidence: 1.0,
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
    for observation in observations {
        let reference = observation.credential_ref();
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
            confidence: 1.0,
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
        Some(ProviderKind::Ollama) => None,
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
                            confidence: 1.0,
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
                confidence: 1.0,
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

fn store_credential_secret(
    provider: CredentialProvider,
    credential_ref: &str,
    raw_value: &str,
) -> Result<(), String> {
    if let Some(path) = test_store_path() {
        return test_store_write(&path, provider, credential_ref, raw_value);
    }
    store_credential_secret_native(provider, credential_ref, raw_value)
}

fn load_credential_secret(
    provider: CredentialProvider,
    credential_ref: &str,
) -> Result<String, String> {
    if let Some(path) = test_store_path() {
        return test_store_read(&path, provider, credential_ref);
    }
    load_credential_secret_native(provider, credential_ref)
}

fn test_store_path() -> Option<PathBuf> {
    std::env::var_os(TEST_STORE_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

fn test_store_write(
    path: &PathBuf,
    provider: CredentialProvider,
    credential_ref: &str,
    raw_value: &str,
) -> Result<(), String> {
    let _guard = test_store_lock()
        .lock()
        .map_err(|_| "credential test store lock poisoned".to_string())?;
    let mut map = test_store_load(path)?;
    map.insert(
        keychain_account(provider, credential_ref),
        raw_value.to_string(),
    );
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create credential test store dir: {e}"))?;
    }
    let json = serde_json::to_string_pretty(&map)
        .map_err(|e| format!("serialize credential test store: {e}"))?;
    std::fs::write(path, json).map_err(|e| format!("write credential test store: {e}"))
}

fn test_store_read(
    path: &PathBuf,
    provider: CredentialProvider,
    credential_ref: &str,
) -> Result<String, String> {
    let _guard = test_store_lock()
        .lock()
        .map_err(|_| "credential test store lock poisoned".to_string())?;
    let map = test_store_load(path)?;
    let account = keychain_account(provider, credential_ref);
    map.get(&account)
        .cloned()
        .ok_or_else(|| format!("credential reference not found in test store: {account}"))
}

fn test_store_lock() -> &'static Mutex<()> {
    TEST_STORE_LOCK.get_or_init(|| Mutex::new(()))
}

fn test_store_load(path: &PathBuf) -> Result<HashMap<String, String>, String> {
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("read credential test store: {e}"))?;
    if text.trim().is_empty() {
        return Ok(HashMap::new());
    }
    serde_json::from_str(&text).map_err(|e| format!("parse credential test store: {e}"))
}

#[cfg(target_os = "macos")]
fn store_credential_secret_native(
    provider: CredentialProvider,
    credential_ref: &str,
    raw_value: &str,
) -> Result<(), String> {
    use security_framework::os::macos::keychain::SecKeychain;

    let keychain = SecKeychain::default().map_err(|e| format!("open default keychain: {e}"))?;
    keychain
        .set_generic_password(
            KEYCHAIN_SERVICE,
            &keychain_account(provider, credential_ref),
            raw_value.as_bytes(),
        )
        .map_err(|e| format!("write credential to keychain: {e}"))
}

#[cfg(not(target_os = "macos"))]
fn store_credential_secret_native(
    _provider: CredentialProvider,
    _credential_ref: &str,
    _raw_value: &str,
) -> Result<(), String> {
    Err("credential keychain storage is only implemented on macOS".to_string())
}

#[cfg(target_os = "macos")]
fn load_credential_secret_native(
    provider: CredentialProvider,
    credential_ref: &str,
) -> Result<String, String> {
    use security_framework::os::macos::keychain::SecKeychain;

    let keychain = SecKeychain::default().map_err(|e| format!("open default keychain: {e}"))?;
    let (password, _) = keychain
        .find_generic_password(
            KEYCHAIN_SERVICE,
            &keychain_account(provider, credential_ref),
        )
        .map_err(|e| format!("read credential from keychain: {e}"))?;
    String::from_utf8(password.as_ref().to_vec())
        .map_err(|e| format!("credential in keychain is not UTF-8: {e}"))
}

#[cfg(not(target_os = "macos"))]
fn load_credential_secret_native(
    _provider: CredentialProvider,
    _credential_ref: &str,
) -> Result<String, String> {
    Err("credential keychain storage is only implemented on macOS".to_string())
}

#[cfg(test)]
mod tests;
