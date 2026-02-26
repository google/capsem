/// OpenAI provider: handles /v1/responses and /v1/chat/completions requests.
///
/// Key injection: Authorization: Bearer header.
/// Upstream: https://api.openai.com
use super::provider::{Provider, ProviderKind};

pub struct OpenAiProvider;

impl Provider for OpenAiProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::OpenAi
    }

    fn upstream_base_url(&self) -> &str {
        "https://api.openai.com"
    }

    fn inject_key(
        &self,
        builder: reqwest::RequestBuilder,
        api_key: &str,
    ) -> reqwest::RequestBuilder {
        builder.header("authorization", format!("Bearer {api_key}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upstream_url_responses() {
        let p = OpenAiProvider;
        assert_eq!(
            p.upstream_url("/v1/responses", None),
            "https://api.openai.com/v1/responses"
        );
    }

    #[test]
    fn upstream_url_chat_completions() {
        let p = OpenAiProvider;
        assert_eq!(
            p.upstream_url("/v1/chat/completions", None),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn kind_is_openai() {
        assert_eq!(OpenAiProvider.kind(), ProviderKind::OpenAi);
    }
}
