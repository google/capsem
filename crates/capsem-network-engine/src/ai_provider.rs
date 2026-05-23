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

const LOCAL_BUILTIN_TOOL_NAMES: &[&str] = &["fetch_http", "grep_http", "http_headers"];

pub fn is_local_builtin_tool(name: &str) -> bool {
    LOCAL_BUILTIN_TOOL_NAMES.contains(&name)
}

/// Classify a model-emitted tool call's origin from its name.
pub fn tool_origin(name: &str) -> &'static str {
    if is_local_builtin_tool(name) {
        "local"
    } else if name.contains("__") {
        "mcp_proxy"
    } else {
        "native"
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

    #[test]
    fn tool_origin_classifies_local_mcp_and_native_tools() {
        assert_eq!(tool_origin("fetch_http"), "local");
        assert_eq!(tool_origin("grep_http"), "local");
        assert_eq!(tool_origin("http_headers"), "local");
        assert_eq!(tool_origin("github__list_issues"), "mcp_proxy");
        assert_eq!(tool_origin("write_file"), "native");
    }
}
