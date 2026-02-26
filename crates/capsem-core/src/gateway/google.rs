/// Google Gemini provider: handles /v1beta/models/* requests.
///
/// Key injection: ?key= query parameter.
/// Upstream: https://generativelanguage.googleapis.com
use super::provider::{Provider, ProviderKind};

pub struct GoogleProvider;

impl Provider for GoogleProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Google
    }

    fn upstream_base_url(&self) -> &str {
        "https://generativelanguage.googleapis.com"
    }

    /// Google uses query param for API key, so we override upstream_url
    /// to NOT include the key here -- inject_key handles it.
    fn upstream_url(&self, path: &str, query: Option<&str>) -> String {
        let base = self.upstream_base_url();
        match query {
            Some(q) => format!("{base}{path}?{q}"),
            None => format!("{base}{path}"),
        }
    }

    fn inject_key(
        &self,
        builder: reqwest::RequestBuilder,
        api_key: &str,
    ) -> reqwest::RequestBuilder {
        builder.query(&[("key", api_key)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upstream_url_stream_generate() {
        let p = GoogleProvider;
        assert_eq!(
            p.upstream_url(
                "/v1beta/models/gemini-2.5-pro:streamGenerateContent",
                None
            ),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-pro:streamGenerateContent"
        );
    }

    #[test]
    fn upstream_url_generate_content() {
        let p = GoogleProvider;
        assert_eq!(
            p.upstream_url("/v1beta/models/gemini-2.5-flash:generateContent", None),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent"
        );
    }

    #[test]
    fn upstream_url_with_existing_query() {
        let p = GoogleProvider;
        assert_eq!(
            p.upstream_url(
                "/v1beta/models/gemini-2.5-pro:streamGenerateContent",
                Some("alt=sse")
            ),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-pro:streamGenerateContent?alt=sse"
        );
    }

    #[test]
    fn kind_is_google() {
        assert_eq!(GoogleProvider.kind(), ProviderKind::Google);
    }
}
