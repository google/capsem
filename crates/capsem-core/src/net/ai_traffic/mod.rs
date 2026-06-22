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

use std::collections::{HashMap, VecDeque};

pub use provider::{ModelProtocol, Provider, ProviderKind};

/// Tracks in-flight traces: maps pending tool call_ids to their trace_id.
///
/// A trace represents one agent turn: starts with a fresh prompt (no tool
/// responses), chains through ToolUse -> tool_response -> next_call cycles,
/// and ends when the stop reason is not ToolUse (e.g. EndTurn, MaxTokens).
pub struct TraceState {
    /// Maps a pending tool call_id to the trace_id it belongs to.
    pending: HashMap<String, String>,
    /// Maps workspace-relative file paths mentioned by model tool-call
    /// arguments to the trace_id that produced the tool call.
    file_hints: HashMap<String, String>,
    file_hint_order: VecDeque<(String, String)>,
}

const MAX_FILE_HINTS: usize = 4096;

impl Default for TraceState {
    fn default() -> Self {
        Self::new()
    }
}

impl TraceState {
    pub fn new() -> Self {
        Self {
            pending: HashMap::new(),
            file_hints: HashMap::new(),
            file_hint_order: VecDeque::new(),
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

    /// Register workspace file paths found in model-emitted tool-call
    /// arguments. The fs monitor later uses this to attribute ordinary
    /// workspace writes to the model/tool trace that caused them.
    pub fn register_tool_file_hints<'a>(
        &mut self,
        trace_id: &str,
        arguments: impl IntoIterator<Item = &'a str>,
    ) {
        for arguments in arguments {
            for path in extract_workspace_file_hints(arguments) {
                self.file_hints.insert(path.clone(), trace_id.to_string());
                self.file_hint_order.push_back((path, trace_id.to_string()));
                self.trim_file_hints();
            }
        }
    }

    /// Look up a trace_id for a workspace-relative file path.
    pub fn lookup_file_path(&self, path: &str) -> Option<String> {
        let path = normalize_workspace_path_hint(path)?;
        self.file_hints.get(&path).cloned()
    }

    /// Remove all pending call_ids for a completed trace (called when
    /// stop_reason is not ToolUse, meaning the trace is done).
    pub fn complete_trace(&mut self, trace_id: &str) {
        self.pending.retain(|_, v| v != trace_id);
    }

    fn trim_file_hints(&mut self) {
        while self.file_hint_order.len() > MAX_FILE_HINTS {
            if let Some((path, trace_id)) = self.file_hint_order.pop_front() {
                if self.file_hints.get(&path) == Some(&trace_id) {
                    self.file_hints.remove(&path);
                }
            }
        }
    }
}

fn extract_workspace_file_hints(arguments: &str) -> Vec<String> {
    let mut paths = Vec::new();
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(arguments) {
        collect_json_file_hints(&json, &mut paths);
    }
    for token in arguments
        .split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '`' | ';' | ',' | ')' | '('))
    {
        if let Some(path) = normalize_workspace_path_hint(token) {
            paths.push(path);
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

fn collect_json_file_hints(value: &serde_json::Value, paths: &mut Vec<String>) {
    match value {
        serde_json::Value::String(value) => {
            if let Some(path) = normalize_workspace_path_hint(value) {
                paths.push(path);
            }
            for token in value.split(|c: char| {
                c.is_whitespace() || matches!(c, '"' | '\'' | '`' | ';' | ',' | ')' | '(')
            }) {
                if let Some(path) = normalize_workspace_path_hint(token) {
                    paths.push(path);
                }
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                collect_json_file_hints(value, paths);
            }
        }
        serde_json::Value::Object(values) => {
            for value in values.values() {
                collect_json_file_hints(value, paths);
            }
        }
        _ => {}
    }
}

fn normalize_workspace_path_hint(raw: &str) -> Option<String> {
    let trimmed = raw
        .trim()
        .trim_matches(|c: char| matches!(c, '"' | '\'' | '`' | '<' | '>'))
        .trim_end_matches(['.', ',', ';', ':']);
    if trimmed.is_empty() {
        return None;
    }
    let relative = trimmed
        .strip_prefix("/root/")
        .or_else(|| trimmed.strip_prefix("/workspace/"))
        .or_else(|| trimmed.strip_prefix("./"))
        .unwrap_or(trimmed);
    if relative.starts_with('/') || relative.is_empty() || relative.contains("..") {
        return None;
    }
    Some(relative.to_string())
}

#[cfg(test)]
mod tests;
