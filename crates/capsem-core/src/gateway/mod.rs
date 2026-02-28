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
pub mod ai_body;
pub mod anthropic;
pub mod events;
pub mod google;
pub mod openai;
pub mod pricing;
pub mod provider;
pub mod request_parser;
pub mod server;
pub mod sse;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use capsem_logger::DbWriter;

pub use provider::{Provider, ProviderKind};
pub use server::router;

/// Tracks in-flight traces: maps pending tool call_ids to their trace_id.
///
/// A trace represents one agent turn: starts with a fresh prompt (no tool
/// responses), chains through ToolUse -> tool_response -> next_call cycles,
/// and ends when the stop reason is not ToolUse (e.g. EndTurn, MaxTokens).
pub struct TraceState {
    /// Maps a pending tool call_id to the trace_id it belongs to.
    pending: HashMap<String, String>,
}

impl Default for TraceState {
    fn default() -> Self {
        Self::new()
    }
}

impl TraceState {
    pub fn new() -> Self {
        Self {
            pending: HashMap::new(),
        }
    }

    /// Look up an existing trace_id from the call_ids of tool responses
    /// in the current request. Returns the first match found.
    pub fn lookup(&self, call_ids: &[String]) -> Option<String> {
        for id in call_ids {
            if let Some(trace_id) = self.pending.get(id) {
                return Some(trace_id.clone());
            }
        }
        None
    }

    /// Register new tool call_ids as belonging to a trace (called when
    /// the model's stop_reason is ToolUse).
    pub fn register_tool_calls(&mut self, trace_id: &str, call_ids: &[String]) {
        for id in call_ids {
            self.pending.insert(id.clone(), trace_id.to_string());
        }
    }

    /// Remove all pending call_ids for a completed trace (called when
    /// stop_reason is not ToolUse, meaning the trace is done).
    pub fn complete_trace(&mut self, trace_id: &str) {
        self.pending.retain(|_, v| v != trace_id);
    }
}

/// Configuration for the AI gateway, shared across all handler invocations.
pub struct GatewayConfig {
    pub anthropic_api_key: Option<String>,
    pub openai_api_key: Option<String>,
    pub google_api_key: Option<String>,
    pub db: Arc<DbWriter>,
    pub http_client: reqwest::Client,
    pub pricing: pricing::PricingTable,
    pub trace_state: Mutex<TraceState>,
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
    pub fn from_env(db: Arc<DbWriter>) -> Self {
        Self {
            anthropic_api_key: std::env::var("ANTHROPIC_API_KEY").ok(),
            openai_api_key: std::env::var("OPENAI_API_KEY").ok(),
            google_api_key: std::env::var("GEMINI_API_KEY")
                .or_else(|_| std::env::var("GOOGLE_API_KEY"))
                .ok(),
            db,
            http_client: reqwest::Client::new(),
            pricing: pricing::PricingTable::load(),
            trace_state: Mutex::new(TraceState::new()),
        }
    }

    /// Create a config from `~/.capsem/user.toml` settings (the canonical
    /// source of API keys for the capsem app).
    pub fn from_capsem_settings(db: Arc<DbWriter>) -> Self {
        use crate::net::policy_config::{load_settings_files, resolve_settings};

        let (user, corp) = load_settings_files();
        let resolved = resolve_settings(&user, &corp);

        let get_key = |id: &str| -> Option<String> {
            resolved
                .iter()
                .find(|s| s.id == id)
                .and_then(|s| s.effective_value.as_text())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
        };

        Self {
            anthropic_api_key: get_key("ai.anthropic.api_key"),
            openai_api_key: get_key("ai.openai.api_key"),
            google_api_key: get_key("ai.google.api_key"),
            db,
            http_client: reqwest::Client::new(),
            pricing: pricing::PricingTable::load(),
            trace_state: Mutex::new(TraceState::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_state_new_trace_on_no_match() {
        let state = TraceState::new();
        assert!(state.lookup(&["call_1".to_string()]).is_none());
        assert!(state.lookup(&[]).is_none());
    }

    #[test]
    fn trace_state_register_and_lookup() {
        let mut state = TraceState::new();
        state.register_tool_calls("trace_A", &["call_1".to_string(), "call_2".to_string()]);

        assert_eq!(state.lookup(&["call_1".to_string()]).as_deref(), Some("trace_A"));
        assert_eq!(state.lookup(&["call_2".to_string()]).as_deref(), Some("trace_A"));
        assert!(state.lookup(&["call_3".to_string()]).is_none());
    }

    #[test]
    fn trace_state_complete_cleans_up() {
        let mut state = TraceState::new();
        state.register_tool_calls("trace_A", &["call_1".to_string()]);
        assert!(state.lookup(&["call_1".to_string()]).is_some());

        state.complete_trace("trace_A");
        assert!(state.lookup(&["call_1".to_string()]).is_none());
    }

    #[test]
    fn trace_state_concurrent_traces_isolated() {
        let mut state = TraceState::new();
        state.register_tool_calls("trace_A", &["call_A1".to_string()]);
        state.register_tool_calls("trace_B", &["call_B1".to_string()]);

        assert_eq!(state.lookup(&["call_A1".to_string()]).as_deref(), Some("trace_A"));
        assert_eq!(state.lookup(&["call_B1".to_string()]).as_deref(), Some("trace_B"));

        // Complete trace_A, trace_B remains.
        state.complete_trace("trace_A");
        assert!(state.lookup(&["call_A1".to_string()]).is_none());
        assert_eq!(state.lookup(&["call_B1".to_string()]).as_deref(), Some("trace_B"));
    }

    #[test]
    fn trace_state_multiple_tool_calls_same_trace() {
        let mut state = TraceState::new();
        let calls: Vec<String> = (0..3).map(|i| format!("call_{i}")).collect();
        state.register_tool_calls("trace_X", &calls);

        for call in &calls {
            assert_eq!(
                state.lookup(&[call.clone()]).as_deref(),
                Some("trace_X"),
            );
        }
    }
}
