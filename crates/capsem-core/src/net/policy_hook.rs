//! External Policy Hook runtime.
//!
//! This module owns the host-side callout contract: validate endpoint
//! security, cap request/response bodies, decode strict Spec0 JSON, and emit
//! a session DB audit row for every decision attempt. Policy integration can
//! call this from MCP/HTTP/DNS/model hooks without duplicating the hardening.

use std::future::Future;
use std::net::IpAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use capsem_logger::events::PolicyHookEvent;
use capsem_logger::writer::{DbWriter, WriteOp};
use futures::StreamExt;
use reqwest::Url;
use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

use crate::net::policy_hook_spec::{
    policy_hook_schema_hash, HookDecision, HookDecisionRequest, HookDecisionResponse,
    POLICY_HOOK_SPEC_VERSION,
};

const DEFAULT_TIMEOUT_MS: u64 = 2_000;
const DEFAULT_BODY_CAP_BYTES: usize = 256 * 1024;

#[derive(Debug, Error)]
pub enum PolicyHookError {
    #[error("hook endpoint URL is invalid: {0}")]
    InvalidUrl(String),
    #[error("hook endpoint must use HTTPS outside localhost")]
    InsecureEndpoint,
    #[error("hook endpoint requires bearer auth outside localhost")]
    MissingAuth,
    #[error("hook request body exceeds cap: {actual} > {cap} bytes")]
    RequestTooLarge { actual: usize, cap: usize },
    #[error("hook response body exceeds cap: {actual} > {cap} bytes")]
    ResponseTooLarge { actual: usize, cap: usize },
    #[error("hook transport failed: {0}")]
    Transport(String),
    #[error("hook returned HTTP {0}")]
    HttpStatus(u16),
    #[error("hook response schema invalid: {0}")]
    Schema(String),
    #[error("hook request spec_version mismatch: {0}")]
    SpecVersion(String),
}

#[derive(Debug, Clone)]
pub struct PolicyHookEndpoint {
    pub id: String,
    pub decision_url: String,
    pub bearer_token: Option<String>,
    pub timeout_ms: u64,
    pub body_cap_bytes: usize,
    pub allow_insecure_localhost: bool,
    pub fail_closed_decision: HookDecision,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PolicyHookEndpointConfig {
    pub id: String,
    pub decision_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bearer_token: Option<String>,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_body_cap_bytes")]
    pub body_cap_bytes: usize,
    #[serde(default = "default_allow_insecure_localhost")]
    pub allow_insecure_localhost: bool,
    #[serde(
        default = "default_fail_closed_decision",
        deserialize_with = "deserialize_fail_closed_decision"
    )]
    pub fail_closed_decision: HookDecision,
}

fn default_timeout_ms() -> u64 {
    DEFAULT_TIMEOUT_MS
}

fn default_body_cap_bytes() -> usize {
    DEFAULT_BODY_CAP_BYTES
}

fn default_allow_insecure_localhost() -> bool {
    true
}

fn default_fail_closed_decision() -> HookDecision {
    HookDecision::Block
}

fn sanitized_fail_closed_decision(decision: HookDecision) -> HookDecision {
    match decision {
        HookDecision::Block | HookDecision::Ask => decision,
        HookDecision::Allow | HookDecision::Rewrite => HookDecision::Block,
    }
}

fn deserialize_fail_closed_decision<'de, D>(deserializer: D) -> Result<HookDecision, D::Error>
where
    D: Deserializer<'de>,
{
    let decision = HookDecision::deserialize(deserializer)?;
    match decision {
        HookDecision::Block | HookDecision::Ask => Ok(decision),
        HookDecision::Allow | HookDecision::Rewrite => Err(serde::de::Error::custom(
            "fail_closed_decision must be block or ask",
        )),
    }
}

impl PolicyHookEndpoint {
    pub fn new(id: impl Into<String>, decision_url: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            decision_url: decision_url.into(),
            bearer_token: None,
            timeout_ms: DEFAULT_TIMEOUT_MS,
            body_cap_bytes: DEFAULT_BODY_CAP_BYTES,
            allow_insecure_localhost: true,
            fail_closed_decision: HookDecision::Block,
        }
    }

    pub fn from_config(config: PolicyHookEndpointConfig) -> Self {
        Self {
            id: config.id,
            decision_url: config.decision_url,
            bearer_token: config.bearer_token,
            timeout_ms: config.timeout_ms,
            body_cap_bytes: config.body_cap_bytes,
            allow_insecure_localhost: config.allow_insecure_localhost,
            fail_closed_decision: sanitized_fail_closed_decision(config.fail_closed_decision),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_millis(self.timeout_ms)
    }

    fn fail_closed_response(
        &self,
        request: &HookDecisionRequest,
        error: &PolicyHookError,
    ) -> HookDecisionResponse {
        let decision = sanitized_fail_closed_decision(self.fail_closed_decision);
        HookDecisionResponse {
            decision,
            decision_id: Some(request.decision_id.clone()),
            rule_id: Some(format!("hook.{}.fail_closed", self.id)),
            priority: None,
            reason: Some(error.to_string()),
            ttl_ms: None,
            rewrite_target: None,
            rewrite_value: None,
            redactions: Vec::new(),
            audit_tags: vec!["policy_hook_error".to_string()],
        }
    }
}

#[derive(Debug, Clone)]
pub struct PolicyHookOutcome {
    pub response: HookDecisionResponse,
    pub failed_closed: bool,
    pub error: Option<String>,
    pub latency_ms: u64,
}

#[derive(Clone)]
pub struct PolicyHookClient {
    transport: Arc<dyn PolicyHookTransport>,
}

impl Default for PolicyHookClient {
    fn default() -> Self {
        Self::new()
    }
}

impl PolicyHookClient {
    pub fn new() -> Self {
        Self {
            transport: Arc::new(ReqwestPolicyHookTransport::default()),
        }
    }

    #[cfg(test)]
    fn with_transport(transport: Arc<dyn PolicyHookTransport>) -> Self {
        Self { transport }
    }

    pub async fn decide(
        &self,
        endpoint: &PolicyHookEndpoint,
        request: &HookDecisionRequest,
        writer: Option<&DbWriter>,
    ) -> PolicyHookOutcome {
        let start = Instant::now();
        let result = self.call(endpoint, request).await;
        let latency_ms = start.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        let outcome = match result {
            Ok(response) => PolicyHookOutcome {
                response,
                failed_closed: false,
                error: None,
                latency_ms,
            },
            Err(error) => PolicyHookOutcome {
                response: endpoint.fail_closed_response(request, &error),
                failed_closed: true,
                error: Some(error.to_string()),
                latency_ms,
            },
        };
        audit_outcome(endpoint, request, &outcome, writer).await;
        outcome
    }

    async fn call(
        &self,
        endpoint: &PolicyHookEndpoint,
        request: &HookDecisionRequest,
    ) -> Result<HookDecisionResponse, PolicyHookError> {
        if request.spec_version != POLICY_HOOK_SPEC_VERSION {
            return Err(PolicyHookError::SpecVersion(request.spec_version.clone()));
        }
        request
            .validate_semantics()
            .map_err(PolicyHookError::Schema)?;

        let url = validate_endpoint(endpoint)?;
        let body =
            serde_json::to_vec(request).map_err(|err| PolicyHookError::Schema(err.to_string()))?;
        if body.len() > endpoint.body_cap_bytes {
            return Err(PolicyHookError::RequestTooLarge {
                actual: body.len(),
                cap: endpoint.body_cap_bytes,
            });
        }

        let response = self
            .transport
            .post(
                url,
                endpoint.timeout(),
                endpoint
                    .bearer_token
                    .as_deref()
                    .filter(|token| !token.is_empty())
                    .map(str::to_string),
                endpoint.body_cap_bytes,
                body,
            )
            .await
            .map_err(|err| match err {
                PolicyHookError::Transport(message) => PolicyHookError::Transport(message),
                err => err,
            })?;
        if !(200..300).contains(&response.status) {
            return Err(PolicyHookError::HttpStatus(response.status));
        }

        if response.body.len() > endpoint.body_cap_bytes {
            return Err(PolicyHookError::ResponseTooLarge {
                actual: response.body.len(),
                cap: endpoint.body_cap_bytes,
            });
        }
        decode_hook_response(&response.body)
    }
}

struct PolicyHookHttpResponse {
    status: u16,
    body: Vec<u8>,
}

trait PolicyHookTransport: Send + Sync {
    fn post<'a>(
        &'a self,
        url: Url,
        timeout: Duration,
        bearer_token: Option<String>,
        body_cap_bytes: usize,
        body: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<PolicyHookHttpResponse, PolicyHookError>> + Send + 'a>>;
}

#[derive(Default)]
struct ReqwestPolicyHookTransport {
    client: reqwest::Client,
}

impl PolicyHookTransport for ReqwestPolicyHookTransport {
    fn post<'a>(
        &'a self,
        url: Url,
        timeout: Duration,
        bearer_token: Option<String>,
        body_cap_bytes: usize,
        body: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<PolicyHookHttpResponse, PolicyHookError>> + Send + 'a>>
    {
        Box::pin(async move {
            let mut builder = self
                .client
                .post(url)
                .timeout(timeout)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body);
            if let Some(token) = bearer_token {
                builder = builder.bearer_auth(token);
            }

            let response = builder
                .send()
                .await
                .map_err(|err| PolicyHookError::Transport(err.to_string()))?;
            let status = response.status().as_u16();
            let mut body = Vec::new();
            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|err| PolicyHookError::Transport(err.to_string()))?;
                let actual = body.len().saturating_add(chunk.len());
                if actual > body_cap_bytes {
                    return Err(PolicyHookError::ResponseTooLarge {
                        actual,
                        cap: body_cap_bytes,
                    });
                }
                body.extend_from_slice(&chunk);
            }
            Ok(PolicyHookHttpResponse { status, body })
        })
    }
}

fn decode_hook_response(bytes: &[u8]) -> Result<HookDecisionResponse, PolicyHookError> {
    let response = serde_json::from_slice::<HookDecisionResponse>(bytes)
        .map_err(|err| PolicyHookError::Schema(err.to_string()))?;
    response
        .validate_semantics()
        .map_err(PolicyHookError::Schema)?;
    Ok(response)
}

fn validate_endpoint(endpoint: &PolicyHookEndpoint) -> Result<Url, PolicyHookError> {
    let url = Url::parse(&endpoint.decision_url)
        .map_err(|err| PolicyHookError::InvalidUrl(err.to_string()))?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(PolicyHookError::InvalidUrl(
            "hook endpoint URL must not contain userinfo".to_string(),
        ));
    }
    let local = is_loopback_hook_url(&url);
    let https = url.scheme() == "https";
    let local_http = endpoint.allow_insecure_localhost && local && url.scheme() == "http";
    if !https && !local_http {
        return Err(PolicyHookError::InsecureEndpoint);
    }
    let has_auth = endpoint
        .bearer_token
        .as_deref()
        .is_some_and(|token| !token.is_empty());
    if !local && !has_auth {
        return Err(PolicyHookError::MissingAuth);
    }
    Ok(url)
}

fn is_loopback_hook_url(url: &Url) -> bool {
    match url.host_str() {
        Some("localhost") => true,
        Some(host) => {
            let host = host
                .strip_prefix('[')
                .and_then(|value| value.strip_suffix(']'))
                .unwrap_or(host);
            host.parse::<IpAddr>().is_ok_and(|addr| addr.is_loopback())
        }
        None => false,
    }
}

async fn audit_outcome(
    endpoint: &PolicyHookEndpoint,
    request: &HookDecisionRequest,
    outcome: &PolicyHookOutcome,
    writer: Option<&DbWriter>,
) {
    let Some(writer) = writer else {
        return;
    };
    let response = &outcome.response;
    let fallback = sanitized_fail_closed_decision(endpoint.fail_closed_decision);
    let event = PolicyHookEvent {
        timestamp: SystemTime::now(),
        endpoint_id: endpoint.id.clone(),
        spec_version: POLICY_HOOK_SPEC_VERSION.to_string(),
        spec_hash: policy_hook_schema_hash(),
        decision_id: response
            .decision_id
            .clone()
            .or_else(|| Some(request.decision_id.clone())),
        callback: request.on.as_str().to_string(),
        decision: (!outcome.failed_closed).then(|| response.decision.as_str().to_string()),
        rule_id: response.rule_id.clone(),
        reason: response.reason.clone(),
        latency_ms: outcome.latency_ms,
        status: if outcome.failed_closed {
            "error".to_string()
        } else {
            response.decision.audit_status().to_string()
        },
        error: outcome.error.clone(),
        fallback: outcome.failed_closed.then(|| fallback.as_str().to_string()),
        audit_tags: response.audit_tags.clone(),
        trace_id: request.trace_id.clone(),
        session_id: request.session_id.clone(),
    };
    writer.write(WriteOp::PolicyHookEvent(event)).await;
}

#[cfg(test)]
mod tests;
