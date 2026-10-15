//! Where a built-in HTTP tool is allowed to connect.
//!
//! The security rules judge `http.host`, but the socket goes to whatever
//! that name resolves to, and the guest can pick a name that resolves to
//! loopback, link-local or a private range -- or one whose answer changes
//! between the check and the dial (DNS rebinding). So the boundary judges
//! the resolved addresses too, refuses a non-public address that no rule
//! explicitly allows, and pins the connection to the exact addresses it
//! judged.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use super::{authorize_upstream, parse_tool_url, SecurityEnforcementDecision, SecurityPluginConfig, SecurityRuleSet};
use crate::mcp::http_transport::BrokerHttpClient;
use capsem_proto::mcp_worker::{McpHttpAdmission, MCP_ADMISSION_HEADER};
use reqwest::Client;
use std::collections::BTreeMap;

pub(super) struct AuthorizedResponse {
    pub domain: String,
    pub decision: SecurityEnforcementDecision,
    pub response: reqwest::Response,
}

pub(super) enum RequestFailure {
    Denied(String),
    Failed(String),
}

/// The HTTP settings the built-in tools connect with. Redirects are never
/// followed: the boundary evaluated the requested URL, and a 3xx to another
/// host would reach a name it never judged.
#[derive(Clone)]
pub struct BuiltinHttpClient {
    request_timeout: Duration,
    connect_timeout: Duration,
    user_agent: Option<String>,
    broker: Option<(BrokerHttpClient, Client)>,
}

impl std::fmt::Debug for BuiltinHttpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BuiltinHttpClient")
            .field("request_timeout", &self.request_timeout)
            .field("connect_timeout", &self.connect_timeout)
            .field("user_agent", &self.user_agent)
            .field("brokered", &self.broker.is_some())
            .finish()
    }
}

impl BuiltinHttpClient {
    pub fn new(request_timeout: Duration, connect_timeout: Duration) -> Self {
        Self {
            request_timeout,
            connect_timeout,
            user_agent: None,
            broker: None,
        }
    }

    /// Native tools receive policy decisions and HTTP solely from the service.
    pub fn brokered(http: BrokerHttpClient) -> Self {
        let mut client = Self::new(
            Duration::from_secs(capsem_proto::MCP_HTTP_TIMEOUT_SECONDS),
            Duration::from_secs(capsem_proto::MCP_STARTUP_TIMEOUT_SECONDS),
        );
        client.broker = Some((http, Client::new()));
        client
    }

    pub(super) async fn request(
        &self,
        url: &str,
        method: &str,
        security_rules: &SecurityRuleSet,
        plugin_policy: &BTreeMap<String, SecurityPluginConfig>,
    ) -> Result<AuthorizedResponse, RequestFailure> {
        let failed = |error| RequestFailure::Failed(format!("HTTP request failed: {error}"));
        if let Some((broker, builder)) = &self.broker {
            let parsed = parse_tool_url(url).map_err(RequestFailure::Denied)?;
            let domain = parsed.host_str().unwrap_or_default().to_owned();
            let request = builder
                .request(method.parse::<http::Method>().expect("builtin HTTP method"), parsed)
                .timeout(self.request_timeout)
                .build()
                .map_err(|error| failed(error.to_string()))?;
            let mut response = broker
                .execute(request)
                .await
                .map_err(|error| failed(error.to_string()))?;
            if response.headers().get_all(MCP_ADMISSION_HEADER).iter().count() != 1 {
                return Err(RequestFailure::Denied(
                    "missing or ambiguous coordinator HTTP admission".into(),
                ));
            }
            let receipt: McpHttpAdmission = serde_json::from_slice(
                response
                    .headers()
                    .get(MCP_ADMISSION_HEADER)
                    .expect("validated admission count")
                    .as_bytes(),
            )
            .map_err(|_| RequestFailure::Denied("invalid coordinator HTTP admission".into()))?;
            response.headers_mut().remove(MCP_ADMISSION_HEADER);
            let decision = match receipt {
                McpHttpAdmission::Allowed { rule_id, reason } => {
                    let mut decision = SecurityEnforcementDecision::allow();
                    decision.rule_id = rule_id;
                    decision.reason = reason;
                    decision
                }
                McpHttpAdmission::Denied { reason } => return Err(RequestFailure::Denied(reason)),
            };
            return Ok(AuthorizedResponse {
                domain,
                decision,
                response,
            });
        }
        let checked = authorize_upstream(url, method, security_rules, plugin_policy)
            .await
            .map_err(RequestFailure::Denied)?;
        let client = self
            .pinned(&checked.domain, &checked.addresses)
            .map_err(|error| RequestFailure::Failed(format!("HTTP client setup failed: {error}")))?;
        let response = client
            .request(method.parse::<http::Method>().expect("builtin HTTP method"), url)
            .send()
            .await
            .map_err(|error| failed(error.to_string()))?;
        Ok(AuthorizedResponse {
            domain: checked.domain,
            decision: checked.decision,
            response,
        })
    }

    pub fn with_user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = Some(user_agent.into());
        self
    }

    fn builder(&self) -> reqwest::ClientBuilder {
        let builder = Client::builder()
            .timeout(self.request_timeout)
            .connect_timeout(self.connect_timeout)
            .redirect(reqwest::redirect::Policy::none());
        match &self.user_agent {
            Some(user_agent) => builder.user_agent(user_agent.clone()),
            None => builder,
        }
    }

    /// A client whose connections for `host` go only to `addresses`: the
    /// ones the boundary judged. A name that re-resolves elsewhere between
    /// the check and the dial reaches nothing new.
    pub fn pinned(&self, host: &str, addresses: &[SocketAddr]) -> reqwest::Result<Client> {
        self.builder().resolve_to_addrs(host, addresses).build()
    }
}

/// The reason a non-public address is refused when no rule allowed it.
pub fn non_public_refusal(domain: &str, address: IpAddr) -> String {
    format!(
        "HTTP request blocked: {domain} resolves to {address}, a non-public address; \
         only an explicit allow rule may reach it"
    )
}
