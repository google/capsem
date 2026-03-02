/// AI traffic parsing and telemetry: SSE stream parsing, request metadata
/// extraction, and provider-agnostic event normalization for AI provider
/// traffic flowing through the MITM proxy (vsock:5002).
///
/// All AI traffic goes through the MITM proxy, which uses these modules for:
/// - Provider detection and routing (`provider.rs`)
/// - Request body parsing for metadata (`request_parser.rs`)
/// - SSE stream parsing for response events (`sse.rs`, `ai_body.rs`)
/// - Provider-specific SSE parsers (`anthropic.rs`, `openai.rs`, `google.rs`)
/// - Unified event collection and summarization (`events.rs`)
/// - Model pricing estimation (`pricing.rs`)
pub mod ai_body;
pub mod anthropic;
pub mod events;
pub mod google;
pub mod openai;
pub mod pricing;
pub mod provider;
pub mod request_parser;
pub mod sse;

use std::collections::HashMap;

pub use provider::{Provider, ProviderKind};

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
