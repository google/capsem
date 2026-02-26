/// AI audit gateway: proxies LLM API traffic from the sandboxed VM to real
/// upstream providers (Anthropic, OpenAI, Google Gemini).
///
/// The gateway receives plain HTTP from the guest (via vsock:5004), routes by
/// request path to the correct provider, injects real API keys, forwards the
/// request (including SSE streaming), and logs everything to an audit DB.
///
/// Architecture (from overall_plan.md Milestone 6):
///   VM agent -> HTTP POST http://10.0.0.1:8080/v1/messages
///     -> iptables REDIRECT -> vsock-bridge -> vsock:5004
///     -> host gateway (this module)
///     -> inject real API key
///     -> upstream HTTPS to api.anthropic.com
///     -> stream SSE response back to agent
///     -> log to audit DB
pub mod anthropic;
pub mod audit;
pub mod google;
pub mod openai;
pub mod provider;
pub mod server;
pub mod streaming;

use std::sync::{Arc, Mutex};

pub use audit::GatewayDb;
pub use provider::{Provider, ProviderKind};
pub use server::router;

/// Configuration for the AI gateway, shared across all handler invocations.
pub struct GatewayConfig {
    pub anthropic_api_key: Option<String>,
    pub openai_api_key: Option<String>,
    pub google_api_key: Option<String>,
    pub audit_db: Arc<Mutex<GatewayDb>>,
    pub http_client: reqwest::Client,
}

impl GatewayConfig {
    /// Look up the API key for a given provider.
    pub fn api_key_for(&self, kind: ProviderKind) -> Option<&str> {
        match kind {
            ProviderKind::Anthropic => self.anthropic_api_key.as_deref(),
            ProviderKind::OpenAi => self.openai_api_key.as_deref(),
            ProviderKind::Google => self.google_api_key.as_deref(),
        }
    }

    /// Create a config from environment variables (for testing).
    pub fn from_env(audit_db: Arc<Mutex<GatewayDb>>) -> Self {
        Self {
            anthropic_api_key: std::env::var("ANTHROPIC_API_KEY").ok(),
            openai_api_key: std::env::var("OPENAI_API_KEY").ok(),
            google_api_key: std::env::var("GEMINI_API_KEY")
                .or_else(|_| std::env::var("GOOGLE_API_KEY"))
                .ok(),
            audit_db,
            http_client: reqwest::Client::new(),
        }
    }
}
