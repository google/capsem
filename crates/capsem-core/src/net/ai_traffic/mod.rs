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
///
/// # Tool call data paths (3 parallel systems)
///
/// 1. **model_calls.tool_calls** (MITM proxy): every tool_use block in an
///    LLM response is recorded with origin ("native"/"local"/"mcp_proxy")
///    via `provider::tool_origin()`. Linked to model_calls by FK.
/// 2. **mcp_calls** (MITM MCP endpoint, vsock:5002): every guest MCP
///    JSON-RPC request is recorded independently by the framed MCP layer.
/// 3. **net_events** (builtin HTTP tools): `fetch_http`/`grep_http`/
///    `http_headers` emit NetEvents for domain policy enforcement.
///
/// # Correlation gaps (next-gen TODOs)
///
/// - `tool_calls.mcp_call_id` is populated opportunistically when the framed
///   MCP call shares the same trace id and normalized tool name as a model
///   tool-use event. The canonical AI evidence tables carry the richer link
///   status (`linked`, `ambiguous`, `orphan_mcp_execution`, etc.).
/// - `mcp_calls.trace_id` is present, but guest/provider trace propagation can
///   still be partial; unknown linkage must remain explicit rather than being
///   inferred from tool-name heuristics alone.
/// - Builtin tool NetEvents are not linked to their tool_call entries.
/// - `tool_origin()` imports `mcp::builtin_tools::is_builtin_tool()` --
///   cross-module coupling that should be replaced by a shared registry.
pub mod evidence;
pub mod pricing;
pub mod provider;
pub mod request_parser;

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
mod tests;
