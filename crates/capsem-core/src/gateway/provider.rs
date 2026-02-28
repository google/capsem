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
}
