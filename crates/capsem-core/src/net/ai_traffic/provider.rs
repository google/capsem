//! Provider trait and routing: maps inbound request paths to upstream AI
//! providers and handles provider-specific key injection.

pub use capsem_network_engine::ai_provider::{extract_model_from_path, tool_origin, ProviderKind};

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

#[cfg(test)]
mod tests;
