/// AI traffic parsing and telemetry: SSE stream parsing, request metadata
/// extraction, and provider-agnostic event normalization for AI provider
/// traffic flowing through the MITM proxy (vsock:5002).
///
/// All AI traffic goes through the MITM proxy, which uses these modules for:
/// - Typed protocol adapters and legacy path routing (`provider.rs`)
/// - Request body parsing for metadata (`request_parser.rs`)
/// - SSE stream parsing for response events (`sse.rs`, `ai_body.rs`)
/// - Protocol-specific response parsers (`anthropic.rs`, `openai.rs`, `google.rs`)
/// - Unified event collection and summarization (`events.rs`)
/// - Model pricing estimation (`pricing.rs`)
///
/// # Provider identity vs protocol
///
/// Provider identity is settings/profile data (`ai.openai`, `ai.ollama`,
/// custom private gateways). Rust owns typed wire protocol adapters such as
/// OpenAI, Anthropic, Google, and native Ollama. A new OpenAI-compatible
/// endpoint must not need a new Rust enum variant.
///
/// # Tool-call telemetry contract
///
/// Model-native tool calls, observed MCP calls, and builtin network events are
/// separate first-party security events. They are correlated by event IDs,
/// trace IDs, and turn/tool identifiers in the logger-owned session DB; no
/// helper table or MCP-only path is allowed to become the source of truth.
pub mod events;
pub mod pricing;
pub mod provider;
pub mod request_parser;

use std::collections::HashMap;

pub use provider::{ModelProtocol, Provider, ProviderKind};

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

#[cfg(test)]
mod tests;
