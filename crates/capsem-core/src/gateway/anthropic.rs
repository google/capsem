/// Anthropic provider: handles /v1/messages requests.
///
/// Key injection: x-api-key header.
/// Upstream: https://api.anthropic.com
use super::provider::{Provider, ProviderKind};

pub struct AnthropicProvider;

impl Provider for AnthropicProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Anthropic
    }

    fn upstream_base_url(&self) -> &str {
        "https://api.anthropic.com"
    }

    fn inject_key(
        &self,
        builder: reqwest::RequestBuilder,
        api_key: &str,
    ) -> reqwest::RequestBuilder {
        builder.header("x-api-key", api_key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upstream_url_messages() {
        let p = AnthropicProvider;
        assert_eq!(
            p.upstream_url("/v1/messages", None),
            "https://api.anthropic.com/v1/messages"
        );
    }

    #[test]
    fn upstream_url_with_query() {
        let p = AnthropicProvider;
        assert_eq!(
            p.upstream_url("/v1/messages", Some("beta=true")),
            "https://api.anthropic.com/v1/messages?beta=true"
        );
    }

    #[test]
    fn kind_is_anthropic() {
        assert_eq!(AnthropicProvider.kind(), ProviderKind::Anthropic);
    }
}
