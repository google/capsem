//! Pure helpers used by the MITM pipeline: LLM-API path detection,
//! URI splitting, and header formatting.

use crate::credential_broker::{detect_http_credential_with_provider, CredentialObservation};
use crate::net::ai_traffic::provider::{ModelProtocol, ProviderKind};

/// Returns true only for paths that are actual LLM API endpoints
/// (generation, embeddings, images, audio -- anything billed per token/request).
pub(super) fn is_llm_api_path(protocol: ModelProtocol, path: &str) -> bool {
    match protocol {
        ModelProtocol::Anthropic => {
            path.starts_with("/v1/messages") || path.starts_with("/v1/complete")
        }
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
pub(super) fn parse_http_host_target(
    header: Option<&hyper::header::HeaderValue>,
) -> Option<(String, u16)> {
    let raw = header?.to_str().ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Bracketed IPv6 form -- T2.2 doesn't handle it; bail.
    if trimmed.starts_with('[') {
        return None;
    }
    match trimmed.rsplit_once(':') {
        Some((host, port_str)) if !host.is_empty() => {
            let port: u16 = port_str.parse().ok()?;
            Some((host.to_string(), port))
        }
        _ => Some((trimmed.to_string(), 80)),
    }
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
                let digest = blake3::hash(raw);
                let hex = &digest.to_hex()[..12];
                format!("{}: hash:{}", name, hex)
            }
        })
        .collect::<Vec<_>>()
        .join("\r\n");

    FormattedHeaders {
        formatted,
        credential_ref: observations
            .first()
            .map(CredentialObservation::credential_ref),
        observations,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credential_broker::CredentialProvider;

    #[test]
    fn header_formatter_sanitizes_and_emits_broker_observations() {
        let mut headers = hyper::HeaderMap::new();
        headers.insert(
            hyper::header::AUTHORIZATION,
            hyper::header::HeaderValue::from_static("Bearer sk-network-format-secret"),
        );

        let formatted =
            format_headers_for_domain("127.0.0.1", Some(ProviderKind::OpenAi), &headers);

        assert_eq!(formatted.observations.len(), 1);
        assert_eq!(
            formatted.observations[0].provider,
            CredentialProvider::OpenAi
        );
        assert_eq!(
            formatted.observations[0].source,
            "http.header.authorization"
        );
        assert_eq!(
            formatted.observations[0].event_type.as_deref(),
            Some("http.request")
        );
        assert_eq!(
            formatted.credential_ref.as_deref(),
            Some(formatted.observations[0].credential_ref().as_str())
        );
        assert!(formatted.formatted.contains("authorization: hash:"));
        assert!(!formatted.formatted.contains("sk-network-format-secret"));
    }
}
