/// Which AI provider produced or received model traffic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    Anthropic,
    OpenAi,
    Google,
}

impl ProviderKind {
    /// Short name for audit logging and canonical evidence projection.
    pub fn as_str(&self) -> &'static str {
        match self {
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::OpenAi => "openai",
            ProviderKind::Google => "google",
        }
    }
}

/// Extract model name from a Gemini-style URL path.
/// E.g. `/v1beta/models/gemini-2.5-flash-lite:generateContent` -> `gemini-2.5-flash-lite`
pub fn extract_model_from_path(path: &str) -> Option<String> {
    let models_idx = path.find("/models/")?;
    let after = &path[models_idx + 8..];
    let model = after.split(':').next()?;
    if model.is_empty() {
        return None;
    }
    Some(model.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_short_names_are_stable() {
        assert_eq!(ProviderKind::Anthropic.as_str(), "anthropic");
        assert_eq!(ProviderKind::OpenAi.as_str(), "openai");
        assert_eq!(ProviderKind::Google.as_str(), "google");
    }

    #[test]
    fn extract_model_from_gemini_path() {
        assert_eq!(
            extract_model_from_path("/v1beta/models/gemini-2.5-flash-lite:generateContent")
                .as_deref(),
            Some("gemini-2.5-flash-lite")
        );
    }

    #[test]
    fn extract_model_rejects_non_model_path() {
        assert!(extract_model_from_path("/v1/messages").is_none());
    }
}
