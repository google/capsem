//! Provider trait and routing: maps inbound request paths to upstream AI
//! providers and handles provider-specific key injection.

use super::events::ProviderStreamParser;

/// Which AI provider handles this request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    Anthropic,
    OpenAi,
    Google,
}

impl ProviderKind {
    /// Short name for audit logging.
    pub fn as_str(&self) -> &'static str {
        match self {
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::OpenAi => "openai",
            ProviderKind::Google => "google",
        }
    }

    /// Create a new SSE stream parser for this provider.
    pub fn create_parser(&self) -> Box<dyn ProviderStreamParser + Send> {
        match self {
            ProviderKind::Anthropic => Box::new(crate::net::interpreters::anthropic_interpreter::AnthropicStreamParserWithState::new()),
            ProviderKind::OpenAi => Box::new(crate::net::interpreters::openai_interpreter::OpenAiStreamParser::new()),
            ProviderKind::Google => Box::new(crate::net::interpreters::google_interpreter::GoogleStreamParser::new()),
        }
    }
}

/// A provider knows how to build the upstream URL and inject API keys.
pub trait Provider: Send + Sync {
    fn kind(&self) -> ProviderKind;

    /// The upstream base URL (e.g., "https://api.anthropic.com").
    fn upstream_base_url(&self) -> &str;

    /// Build the full upstream URL from the inbound request path and query.
    fn upstream_url(&self, path: &str, query: Option<&str>) -> String {
        let base = self.upstream_base_url();
        match query {
            Some(q) => format!("{base}{path}?{q}"),
            None => format!("{base}{path}"),
        }
    }

    /// Inject the real API key into the outgoing reqwest::RequestBuilder.
    /// Returns the modified builder.
    fn inject_key(
        &self,
        builder: reqwest::RequestBuilder,
        api_key: &str,
    ) -> reqwest::RequestBuilder;
}

/// Determine the provider from the inbound request path.
/// Returns None for paths that don't match any known provider API.
pub fn route_provider(path: &str) -> Option<(ProviderKind, Box<dyn Provider>)> {
    if path.starts_with("/v1/messages") {
        Some((
            ProviderKind::Anthropic,
            Box::new(crate::net::interpreters::anthropic_interpreter::AnthropicProvider),
        ))
    } else if path.starts_with("/v1beta/") {
        Some((
            ProviderKind::Google,
            Box::new(crate::net::interpreters::google_interpreter::GoogleProvider),
        ))
    } else if path.starts_with("/v1/responses") || path.starts_with("/v1/chat/completions") {
        Some((
            ProviderKind::OpenAi,
            Box::new(crate::net::interpreters::openai_interpreter::OpenAiProvider),
        ))
    } else {
        None
    }
}

/// Extract model name from a Gemini-style URL path.
/// E.g. `/v1beta/models/gemini-2.5-flash-lite:generateContent` -> `gemini-2.5-flash-lite`
pub fn extract_model_from_path(path: &str) -> Option<String> {
    // Match pattern: /v.../models/{model}:{action}
    let models_idx = path.find("/models/")?;
    let after = &path[models_idx + 8..]; // skip "/models/"
    let model = after.split(':').next()?;
    if model.is_empty() {
        return None;
    }
    Some(model.to_string())
}

/// Classify a tool call's origin from its name (heuristic).
///
/// - Built-in MCP tools (fetch_http, grep_http, http_headers): "local"
/// - External MCP tools with server__tool namespacing: "mcp_proxy"
/// - Native model tools (write_file, bash, run_shell_command, etc.): "native"
///
/// # Known limitations (next-gen TODOs)
///
/// - **Cross-module import**: calls `mcp::builtin_tools::is_builtin_tool()`,
///   coupling ai_traffic to the MCP module. A shared tool registry would be
///   cleaner but premature until next-gen unifies tool tracking.
/// - **Heuristic-only**: uses `__` as MCP namespace separator. If a native
///   tool name contains `__`, it would be misclassified as mcp_proxy.
/// - **No correlation to mcp_calls**: the `mcp_call_id` column in
///   `tool_calls` is defined but never populated. There is no mechanism to
///   link a model_call's tool_call entry to the corresponding mcp_calls row.
///   Next-gen should propagate a shared call_id or request_id through the
///   guest MCP endpoint.
pub fn tool_origin(name: &str) -> &'static str {
    if crate::mcp::builtin_tools::is_builtin_tool(name) {
        "local"
    } else if name.contains("__") {
        "mcp_proxy"
    } else {
        "native"
    }
}

#[cfg(test)]
mod tests;
