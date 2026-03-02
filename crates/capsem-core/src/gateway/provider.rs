/// Provider trait and routing: maps inbound request paths to upstream AI
/// providers and handles provider-specific key injection.

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
            ProviderKind::Anthropic => Box::new(super::anthropic::AnthropicStreamParserWithState::new()),
            ProviderKind::OpenAi => Box::new(super::openai::OpenAiStreamParser::new()),
            ProviderKind::Google => Box::new(super::google::GoogleStreamParser::new()),
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
            Box::new(super::anthropic::AnthropicProvider),
        ))
    } else if path.starts_with("/v1beta/") {
        Some((
            ProviderKind::Google,
            Box::new(super::google::GoogleProvider),
        ))
    } else if path.starts_with("/v1/responses") || path.starts_with("/v1/chat/completions") {
        Some((
            ProviderKind::OpenAi,
            Box::new(super::openai::OpenAiProvider),
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

/// Determine the origin of a tool call based on its name.
///
/// - Built-in MCP tools (fetch_http, grep_http, http_headers): "mcp"
/// - External MCP tools with server__tool namespacing: "mcp"
/// - Native model tools (write_file, bash, run_shell_command, etc.): "native"
pub fn tool_origin(name: &str) -> &'static str {
    if crate::mcp::builtin_tools::is_builtin_tool(name) || name.contains("__") {
        "mcp"
    } else {
        "native"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_anthropic_messages() {
        let (kind, _) = route_provider("/v1/messages").unwrap();
        assert_eq!(kind, ProviderKind::Anthropic);
    }

    #[test]
    fn route_anthropic_messages_with_query() {
        let (kind, _) = route_provider("/v1/messages?beta=true").unwrap();
        assert_eq!(kind, ProviderKind::Anthropic);
    }

    #[test]
    fn route_openai_responses() {
        let (kind, _) = route_provider("/v1/responses").unwrap();
        assert_eq!(kind, ProviderKind::OpenAi);
    }

    #[test]
    fn route_openai_chat_completions() {
        let (kind, _) = route_provider("/v1/chat/completions").unwrap();
        assert_eq!(kind, ProviderKind::OpenAi);
    }

    #[test]
    fn route_google_gemini() {
        let (kind, _) =
            route_provider("/v1beta/models/gemini-2.5-pro:streamGenerateContent").unwrap();
        assert_eq!(kind, ProviderKind::Google);
    }

    #[test]
    fn route_google_gemini_generate() {
        let (kind, _) =
            route_provider("/v1beta/models/gemini-2.5-pro:generateContent").unwrap();
        assert_eq!(kind, ProviderKind::Google);
    }

    #[test]
    fn route_unknown_returns_none() {
        assert!(route_provider("/v2/something").is_none());
        assert!(route_provider("/health").is_none());
        assert!(route_provider("/").is_none());
    }

    #[test]
    fn provider_kind_as_str() {
        assert_eq!(ProviderKind::Anthropic.as_str(), "anthropic");
        assert_eq!(ProviderKind::OpenAi.as_str(), "openai");
        assert_eq!(ProviderKind::Google.as_str(), "google");
    }

    // -- extract_model_from_path --

    #[test]
    fn extract_model_gemini_stream() {
        assert_eq!(
            extract_model_from_path("/v1beta/models/gemini-2.5-flash:streamGenerateContent"),
            Some("gemini-2.5-flash".to_string())
        );
    }

    #[test]
    fn extract_model_gemini_generate() {
        assert_eq!(
            extract_model_from_path("/v1beta/models/gemini-2.5-pro:generateContent"),
            Some("gemini-2.5-pro".to_string())
        );
    }

    #[test]
    fn extract_model_no_models_segment() {
        assert_eq!(extract_model_from_path("/v1/messages"), None);
    }

    #[test]
    fn extract_model_empty_model() {
        assert_eq!(extract_model_from_path("/v1beta/models/:generateContent"), None);
    }

    // -- tool_origin --

    #[test]
    fn tool_origin_native_tools() {
        assert_eq!(tool_origin("write_file"), "native");
        assert_eq!(tool_origin("bash"), "native");
        assert_eq!(tool_origin("run_shell_command"), "native");
        assert_eq!(tool_origin("read_file"), "native");
    }

    #[test]
    fn tool_origin_builtin_mcp_tools() {
        assert_eq!(tool_origin("fetch_http"), "mcp");
        assert_eq!(tool_origin("grep_http"), "mcp");
        assert_eq!(tool_origin("http_headers"), "mcp");
    }

    #[test]
    fn tool_origin_external_mcp_tools() {
        assert_eq!(tool_origin("github__list_issues"), "mcp");
        assert_eq!(tool_origin("jira__create_ticket"), "mcp");
        assert_eq!(tool_origin("custom_server__my_tool"), "mcp");
    }
}
