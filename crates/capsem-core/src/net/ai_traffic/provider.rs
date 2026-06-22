//! Model provider identity and wire protocol adapters.
//!
//! Provider identity and wire protocol are deliberately separate. A local
//! Ollama endpoint can speak OpenAI or Anthropic-compatible wire protocol,
//! and a rogue endpoint can speak OpenAI protocol without being the OpenAI
//! provider.

use super::events::{LlmEvent, ProviderStreamParser};
use crate::net::parsers::sse_parser::SseEvent;

/// Which model wire protocol/parser handles this request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelProtocol {
    Anthropic,
    OpenAi,
    Google,
    Ollama,
}

impl ModelProtocol {
    /// Short name for audit logging.
    pub fn as_str(&self) -> &'static str {
        match self {
            ModelProtocol::Anthropic => "anthropic",
            ModelProtocol::OpenAi => "openai",
            ModelProtocol::Google => "google",
            ModelProtocol::Ollama => "ollama",
        }
    }

    /// Create a new SSE stream parser for this provider.
    pub fn create_parser(&self) -> Box<dyn ProviderStreamParser + Send> {
        match self {
            ModelProtocol::Anthropic => Box::new(crate::net::interpreters::anthropic_interpreter::AnthropicStreamParserWithState::new()),
            ModelProtocol::OpenAi => Box::new(crate::net::interpreters::openai_interpreter::OpenAiStreamParser::new()),
            ModelProtocol::Google => Box::new(crate::net::interpreters::google_interpreter::GoogleStreamParser::new()),
            ModelProtocol::Ollama => Box::new(NativeOllamaStreamParser),
        }
    }
}

struct NativeOllamaStreamParser;

impl ProviderStreamParser for NativeOllamaStreamParser {
    fn parse_event(&mut self, _sse: &SseEvent) -> Vec<LlmEvent> {
        Vec::new()
    }
}

impl TryFrom<&str> for ModelProtocol {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value.trim().to_ascii_lowercase().as_str() {
            "anthropic" | "claude" => Ok(Self::Anthropic),
            "openai" | "openai_compatible" | "openai-compatible" => Ok(Self::OpenAi),
            "google" | "gemini" => Ok(Self::Google),
            "ollama" => Ok(Self::Ollama),
            other => Err(format!("unknown model protocol '{other}'")),
        }
    }
}

/// Which provider owns this model endpoint for policy and logging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    Unknown,
    Anthropic,
    OpenAi,
    Google,
    Ollama,
}

impl ProviderKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProviderKind::Unknown => "unknown",
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::OpenAi => "openai",
            ProviderKind::Google => "google",
            ProviderKind::Ollama => "ollama",
        }
    }

    pub fn from_provider_id(provider_id: &str) -> Self {
        match provider_id.trim().to_ascii_lowercase().as_str() {
            "anthropic" | "claude" => Self::Anthropic,
            "openai" => Self::OpenAi,
            "google" | "gemini" => Self::Google,
            "ollama" => Self::Ollama,
            _ => Self::Unknown,
        }
    }
}

impl From<ModelProtocol> for ProviderKind {
    fn from(protocol: ModelProtocol) -> Self {
        match protocol {
            ModelProtocol::Anthropic => Self::Anthropic,
            ModelProtocol::OpenAi => Self::OpenAi,
            ModelProtocol::Google => Self::Google,
            ModelProtocol::Ollama => Self::Ollama,
        }
    }
}

/// A provider knows how to build the upstream URL and inject API keys.
pub trait Provider: Send + Sync {
    fn kind(&self) -> ModelProtocol;

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

struct OllamaProvider;

impl Provider for OllamaProvider {
    fn kind(&self) -> ModelProtocol {
        ModelProtocol::Ollama
    }

    fn upstream_base_url(&self) -> &str {
        "http://127.0.0.1:11434"
    }

    fn inject_key(
        &self,
        builder: reqwest::RequestBuilder,
        _api_key: &str,
    ) -> reqwest::RequestBuilder {
        builder
    }
}

/// Determine the provider from the inbound request path.
/// Returns None for paths that don't match any known provider API.
pub fn route_provider(path: &str) -> Option<(ModelProtocol, Box<dyn Provider>)> {
    if path.starts_with("/v1/messages") {
        Some((
            ModelProtocol::Anthropic,
            Box::new(crate::net::interpreters::anthropic_interpreter::AnthropicProvider),
        ))
    } else if path.starts_with("/v1beta/") {
        Some((
            ModelProtocol::Google,
            Box::new(crate::net::interpreters::google_interpreter::GoogleProvider),
        ))
    } else if path.starts_with("/v1/responses") || path.starts_with("/v1/chat/completions") {
        Some((
            ModelProtocol::OpenAi,
            Box::new(crate::net::interpreters::openai_interpreter::OpenAiProvider),
        ))
    } else if path.starts_with("/api/chat") || path.starts_with("/api/generate") {
        Some((ModelProtocol::Ollama, Box::new(OllamaProvider)))
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
/// - **Best-effort correlation**: the canonical tool ledger is `tool_calls`.
///   Model-native rows attach to their `model_calls.id`; MCP-observed rows use
///   `origin = "mcp"` and may be orphan/direct evidence when no model response
///   was visible.
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
