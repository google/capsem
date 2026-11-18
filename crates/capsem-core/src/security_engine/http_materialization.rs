//! The single provider-scoped runtime credential materialization boundary.
use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequestSecurityEvent {
    pub domain: String,
    pub ai_provider: Option<ProviderKind>,
    pub headers: http::HeaderMap,
    pub query: Option<String>,
}

impl HttpRequestSecurityEvent {
    pub fn new(
        domain: impl Into<String>,
        ai_provider: Option<ProviderKind>,
        headers: http::HeaderMap,
        query: Option<String>,
    ) -> Self {
        Self {
            domain: domain.into(),
            ai_provider,
            headers,
            query,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializedHttpRequest {
    pub headers: http::HeaderMap,
    pub query: Option<String>,
    pub credential_ref: Option<String>,
    pub credential_response_rewrites: Vec<crate::credential_broker::CredentialResponseRewrite>,
}

pub fn materialize_http_request_for_upstream(
    event: &SecurityEvent,
) -> Result<MaterializedHttpRequest, SecurityActionError> {
    materialize_http_request_for_upstream_with_credentials(event, &crate::net::proxy_engine::LocalProxyCredentials)
}

pub(crate) fn materialize_http_request_for_upstream_with_credentials(
    event: &SecurityEvent,
    credentials: &dyn crate::net::proxy_engine::ProxyCredentials,
) -> Result<MaterializedHttpRequest, SecurityActionError> {
    let Some(request) = event.http_request.as_ref() else {
        return Err(SecurityActionError::new(
            "security event does not carry an HTTP request",
        ));
    };

    if !event.action_trace.contains(&PolicyActionId::CredentialBrokerSubstitute) {
        if crate::credential_broker::has_brokered_http_references(&request.headers, request.query.as_deref()) {
            return Err(SecurityActionError::new(
                "opaque credential reference requires credential broker substitution",
            ));
        }
        return Ok(MaterializedHttpRequest {
            headers: request.headers.clone(),
            query: request.query.clone(),
            credential_ref: event.credential_ref.clone(),
            credential_response_rewrites: Vec::new(),
        });
    }

    let mut headers = request.headers.clone();
    let BrokeredUpstreamCredentials { credential_ref, query } = credentials
        .substitute_upstream(
            &request.domain,
            request.ai_provider,
            &mut headers,
            request.query.as_deref(),
        )
        .map_err(SecurityActionError::new)?;

    let credential_response_rewrites =
        response_rewrites(request, &headers, query.as_deref(), &event.credential_injections);
    Ok(MaterializedHttpRequest {
        headers,
        query,
        credential_ref: event.credential_ref.clone().or(credential_ref),
        credential_response_rewrites,
    })
}

fn response_rewrites(
    request: &HttpRequestSecurityEvent,
    headers: &http::HeaderMap,
    query: Option<&str>,
    injections: &[crate::credential_broker::CredentialInjection],
) -> Vec<crate::credential_broker::CredentialResponseRewrite> {
    let mut rewrites = Vec::new();
    for injection in injections {
        let raw = injection
            .source
            .strip_prefix("http.header.")
            .and_then(|name| raw_header_substitution(&request.headers, headers, name, &injection.credential_ref))
            .or_else(|| {
                let name = injection.source.strip_prefix("http.query.")?;
                raw_query_substitution(request.query.as_deref()?, query?, name, &injection.credential_ref)
            });
        if let Some(raw_value) = raw {
            let rewrite = crate::credential_broker::CredentialResponseRewrite {
                credential_ref: injection.credential_ref.clone(),
                raw_value,
            };
            if !rewrites.contains(&rewrite) {
                rewrites.push(rewrite);
            }
        }
    }
    rewrites
}

fn raw_header_substitution(
    original_headers: &http::HeaderMap,
    materialized_headers: &http::HeaderMap,
    name: &str,
    credential_ref: &str,
) -> Option<String> {
    original_headers
        .get_all(name)
        .iter()
        .zip(materialized_headers.get_all(name).iter())
        .find_map(|(original, materialized)| {
            raw_header_value_substitution(original.to_str().ok()?, materialized.to_str().ok()?, credential_ref)
        })
}

fn raw_header_value_substitution(original: &str, materialized: &str, credential_ref: &str) -> Option<String> {
    let original = original.trim();
    let materialized = materialized.trim();
    if original == credential_ref {
        return Some(materialized.to_string());
    }
    let original_bearer = original
        .strip_prefix("Bearer ")
        .or_else(|| original.strip_prefix("bearer "))?;
    (original_bearer == credential_ref)
        .then(|| {
            materialized
                .strip_prefix("Bearer ")
                .or_else(|| materialized.strip_prefix("bearer "))
        })
        .flatten()
        .map(str::to_string)
}

pub(super) fn raw_query_substitution(
    original: &str,
    materialized: &str,
    name: &str,
    credential_ref: &str,
) -> Option<String> {
    query_values(original, name)
        .into_iter()
        .zip(query_values(materialized, name))
        .find_map(|(original, materialized)| (original == credential_ref).then_some(materialized))
}

fn query_values(query: &str, expected_name: &str) -> Vec<String> {
    query
        .split('&')
        .filter_map(|part| {
            let (name, value) = part.split_once('=')?;
            (name == expected_name)
                .then(|| crate::credential_broker::percent_decode(value).ok())
                .flatten()
        })
        .collect()
}

pub fn materialize_http_request_for_upstream_after_enforcement(
    event: &SecurityEvent,
    decision: &SecurityEnforcementDecision,
) -> Result<MaterializedHttpRequest, SecurityActionError> {
    if !decision.is_allowed() {
        return Err(SecurityActionError::new(format!(
            "security rule '{}' requires '{}' before HTTP materialization",
            decision.rule_id.as_deref().unwrap_or("unknown"),
            decision.action.as_str()
        )));
    }
    materialize_http_request_for_upstream(event)
}
