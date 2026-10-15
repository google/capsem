//! Pure helpers used by the MITM pipeline: LLM-API path detection,
//! URI splitting, and header formatting.

use crate::credential_broker::{detect_http_credential_with_provider, CredentialObservation};
use crate::net::ai_traffic::provider::{ModelProtocol, ProviderKind};
use crate::net::policy::NetworkMechanics;
use crate::security_engine::{
    IpSecurityEvent, ModelSecurityEvent, RuntimeSecurityEventType, SecurityEvent, TcpSecurityEvent,
};
use std::net::IpAddr;

use super::protocol::Protocol;

pub(super) fn request_can_replay_empty_body(method: &http::Method, headers: &http::HeaderMap) -> bool {
    let no_declared_length = headers
        .get(http::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim() == "0")
        .unwrap_or(true);
    let no_chunked_body = !headers.contains_key(http::header::TRANSFER_ENCODING);
    no_declared_length
        && no_chunked_body
        && matches!(
            *method,
            http::Method::GET | http::Method::HEAD | http::Method::OPTIONS | http::Method::DELETE
        )
}

pub(super) fn is_openai_model_name(model: &str) -> bool {
    let model = model.to_ascii_lowercase();
    model.starts_with("gpt-")
        || model.starts_with("o1")
        || model.starts_with("o3")
        || model.starts_with("o4")
        || model.starts_with("chatgpt-")
}

pub(super) fn is_anthropic_model_name(model: &str) -> bool {
    model.to_ascii_lowercase().starts_with("claude-")
}

pub(super) fn is_google_model_name(model: &str) -> bool {
    let model = model.to_ascii_lowercase();
    model.starts_with("gemini-") || model.starts_with("models/gemini-")
}

pub(super) fn provider_label(provider: Option<ProviderKind>) -> &'static str {
    provider.map(|provider| provider.as_str()).unwrap_or("none")
}

/// Whether a plain-HTTP request may reach `port` on the upstream.
pub(super) fn http_upstream_port_allowed(policy: &NetworkMechanics, protocol: Protocol, port: u16) -> bool {
    if protocol != Protocol::Http || policy.http_upstream_ports.is_empty() {
        return true;
    }
    policy.http_upstream_ports.contains(&port)
}

pub(super) fn current_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

pub(super) fn materialize_collected_response_headers(headers: &mut http::HeaderMap, body_len: usize, is_gzip: bool) {
    if is_gzip {
        headers.remove(http::header::CONTENT_ENCODING);
    }
    headers.remove(http::header::CONTENT_LENGTH);
    headers.remove(http::header::TRANSFER_ENCODING);
    if let Ok(value) = http::HeaderValue::from_str(&body_len.to_string()) {
        headers.insert(http::header::CONTENT_LENGTH, value);
    }
}

/// Returns true only for paths that are actual LLM API endpoints
/// (generation, embeddings, images, audio -- anything billed per token/request).
pub(super) fn is_llm_api_path(protocol: ModelProtocol, path: &str) -> bool {
    match protocol {
        ModelProtocol::Anthropic => path.starts_with("/v1/messages") || path.starts_with("/v1/complete"),
        ModelProtocol::OpenAi => {
            path.starts_with("/v1/chat/completions")
                || path.starts_with("/v1/responses")
                || path.starts_with("/v1/completions")
                || path.starts_with("/v1/embeddings")
                || path.starts_with("/v1/images")
                || path.starts_with("/v1/audio")
        }
        ModelProtocol::Google => {
            path.contains(":generateContent")
                || path.contains(":streamGenerateContent")
                || path.contains(":embedContent")
                || path.contains(":batchEmbedContents")
        }
        ModelProtocol::Ollama => {
            path.starts_with("/api/chat")
                || path.starts_with("/api/generate")
                || path.starts_with("/api/embeddings")
                || path.starts_with("/api/embed")
                || path.starts_with("/v1/chat/completions")
                || path.starts_with("/v1/completions")
                || path.starts_with("/v1/embeddings")
        }
    }
}

/// Split a URI into path and query components.
pub(super) fn split_path_query(uri: &hyper::Uri) -> (String, Option<String>) {
    let path = uri.path().to_string();
    let query = uri.query().map(|q| q.to_string());
    (path, query)
}

/// Parse an HTTP/1.1 `Host` header into `(host, port)`. Used by the
/// plain-HTTP path (T2.2) to derive the connection's authoritative
/// upstream from the inbound request. Defaults to port 80 when the
/// header carries only a host. IPv6-bracketed forms (`[::1]:8080`)
/// are not supported in T2.2 -- the guest's net_proxy doesn't relay
/// IPv6 today.
pub(super) fn parse_http_host_target(header: Option<&hyper::header::HeaderValue>) -> Option<(String, u16)> {
    let raw = header?.to_str().ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Bracketed IPv6 form -- T2.2 doesn't handle it; bail.
    if trimmed.starts_with('[') {
        return None;
    }
    // The host is guest-controlled and this is the plain-HTTP path's only
    // source of upstream identity: hand back the normalized form (lowercase,
    // no DNS-root dots) so policy, dial and telemetry agree, and refuse a
    // value that normalizes to nothing.
    let (host, port) = match trimmed.rsplit_once(':') {
        Some((host, port_str)) if !host.is_empty() => (host, port_str.parse::<u16>().ok()?),
        _ => (trimmed, 80),
    };
    let host = crate::net::hostname::normalize_host(host);
    (!host.is_empty()).then_some((host, port))
}

/// Headers whose values are safe to store verbatim in telemetry logs.
/// Everything else keeps its name but the value is replaced with a short hash.
/// Provider-aware credential handling belongs to the security-engine plugin
/// rail, not this network formatting helper.
const HEADER_ALLOWLIST: &[&str] = &[
    "accept",
    "content-encoding",
    "content-length",
    "content-type",
    "date",
    "host",
    "server",
    "transfer-encoding",
    "user-agent",
];

#[derive(Debug, Clone, PartialEq)]
pub(super) struct FormattedHeaders {
    pub formatted: String,
    pub observations: Vec<CredentialObservation>,
    pub credential_ref: Option<String>,
}

/// Format HTTP headers for telemetry storage.
///
/// Allowlisted headers are stored verbatim. All other headers keep their
/// name but the value is replaced with `hash:<12-char-hex>`. Credential-shaped
/// values also emit broker observations for the security ledger.
pub(super) fn format_headers(headers: &hyper::HeaderMap) -> String {
    format_headers_for_domain("", None, headers).formatted
}

pub(super) fn format_headers_for_domain(
    domain: &str,
    ai_provider: Option<ProviderKind>,
    headers: &hyper::HeaderMap,
) -> FormattedHeaders {
    let provider_hint = ai_provider.map(|provider| match provider {
        ProviderKind::Unknown => ProviderKind::Unknown,
        ProviderKind::Ollama => ProviderKind::OpenAi,
        other => other,
    });
    let mut observations = Vec::new();
    let formatted = headers
        .iter()
        .map(|(name, value)| {
            if HEADER_ALLOWLIST.contains(&name.as_str()) {
                let v = value.to_str().unwrap_or("<binary>");
                format!("{}: {}", name, v)
            } else {
                let raw = value.as_bytes();
                if let Some(observation) =
                    detect_http_credential_with_provider(domain, provider_hint, name.as_str(), raw)
                {
                    observations.push(observation);
                }
                let digest = capsem_crypto::blake3::hash(raw);
                let hex = &digest.to_hex()[..12];
                format!("{}: hash:{}", name, hex)
            }
        })
        .collect::<Vec<_>>()
        .join("\r\n");

    FormattedHeaders {
        formatted,
        credential_ref: observations.first().map(CredentialObservation::credential_ref),
        observations,
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct SecurityBoundaryDecisionFields {
    pub(super) policy_mode: Option<String>,
    pub(super) policy_action: Option<String>,
    pub(super) policy_rule: Option<String>,
    pub(super) policy_reason: Option<String>,
}

impl SecurityBoundaryDecisionFields {
    pub(super) fn from_enforcement(decision: &crate::security_engine::SecurityEnforcementDecision) -> Self {
        Self {
            policy_mode: Some("enforce".to_string()),
            policy_action: Some(decision.action.as_str().to_string()),
            policy_rule: decision.rule_id.clone(),
            policy_reason: decision.reason.clone(),
        }
    }

    pub(super) fn matched_rule(&self, fallback: String) -> String {
        self.policy_rule.clone().unwrap_or(fallback)
    }
}

/// Returns true only when `policy_snapshot` contains an active rule or plugin
/// that can enforce or rewrite based on the HTTP/model response body or status.
/// When false, model responses stream directly through `ChunkDispatchBody`
/// without buffering the entire SSE stream before sending `200 OK` headers.
pub(super) fn policy_requires_buffered_model_response(
    policy_snapshot: &crate::net::proxy_engine::ProxyPolicySnapshot,
) -> bool {
    const RESPONSE_STAGE_FIELDS: &[&str] = &[
        "model.response.body",
        "model.response.valid",
        "http.status",
        "http.body",
    ];
    let has_response_enforcing_rule = policy_snapshot.security_rules().rules().iter().any(|rule| {
        rule.enabled
            && !matches!(rule.action, crate::net::policy_config::SecurityRuleAction::Allow)
            && rule.references_any_field(RESPONSE_STAGE_FIELDS)
    });
    let has_content_plugin = policy_snapshot
        .plugins()
        .get("dummy_pre_eicar")
        .is_some_and(|cfg| !matches!(cfg.mode, crate::net::policy_config::SecurityPluginMode::Disable));
    has_response_enforcing_rule || has_content_plugin
}

pub(super) fn response_body_capture_limit(
    ai_provider: Option<ProviderKind>,
    domain: &str,
    path: &str,
    log_bodies: bool,
    max_body: usize,
    credential_ref: Option<&str>,
) -> usize {
    let cap = super::body_capture_limit(ai_provider, domain, path, log_bodies, max_body);
    if credential_ref.is_some() {
        cap.max(super::CREDENTIAL_BODY_CAPTURE_LIMIT)
    } else {
        cap
    }
}

pub(super) fn model_security_event(
    event_type: RuntimeSecurityEventType,
    provider: ProviderKind,
    model: Option<String>,
    request_body: Option<&[u8]>,
    response_body: Option<&[u8]>,
) -> SecurityEvent {
    SecurityEvent::new(event_type).with_model(ModelSecurityEvent {
        provider: Some(provider.as_str().to_string()),
        name: model,
        request_body: request_body.map(|body| String::from_utf8_lossy(body).to_string()),
        response_body: response_body.map(|body| String::from_utf8_lossy(body).to_string()),
        tool_calls: None,
    })
}

/// Transport facts for the rules: the upstream port, and the address the dial
/// reaches (`UpstreamTarget::judged_ip`), never one read off the guest's name.
pub(super) fn security_event_with_transport(
    mut event: SecurityEvent,
    ip: Option<IpAddr>,
    upstream_port: u16,
) -> SecurityEvent {
    event = event.with_tcp(TcpSecurityEvent {
        port: Some(upstream_port.to_string()),
    });
    if let Some(ip) = ip {
        event = event.with_ip(IpSecurityEvent {
            value: Some(ip.to_string()),
            version: Some(match ip {
                IpAddr::V4(_) => "4".to_string(),
                IpAddr::V6(_) => "6".to_string(),
            }),
        });
    }

    event
}

#[cfg(test)]
mod tests;
