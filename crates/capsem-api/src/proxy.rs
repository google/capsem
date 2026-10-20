use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Start one VM-free, OpenAI-compatible model proxy lease.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
pub struct CreateProxyRequest {
    /// Provider key from the effective built-in/settings/corp policy.
    pub provider: String,
    /// Local address on which the unauthenticated data listener binds.
    pub bind: String,
    /// Requested data-listener port. Zero asks the OS to select one.
    pub port: u16,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
pub struct CreateProxyResponse {
    pub session_id: String,
    pub provider: String,
    pub bind: String,
    pub port: u16,
    /// SDK-compatible URL, including the configured provider path prefix.
    pub base_url: String,
    /// Opaque proof required to renew or stop this lease.
    pub lease_token: String,
    pub lease_expires_unix_ms: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
pub struct ProxyLeaseRequest {
    pub lease_token: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
pub struct ProxyHeartbeatResponse {
    pub session_id: String,
    pub lease_expires_unix_ms: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
pub struct StopProxyResponse {
    pub session_id: String,
    pub stopped: bool,
}
