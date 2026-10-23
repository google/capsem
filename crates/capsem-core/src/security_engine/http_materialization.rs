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
        return Ok(MaterializedHttpRequest {
            headers: request.headers.clone(),
            query: request.query.clone(),
            credential_ref: event.credential_ref.clone(),
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

    Ok(MaterializedHttpRequest {
        headers,
        query,
        credential_ref: event.credential_ref.clone().or(credential_ref),
    })
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
