//! Model provider identity and wire-protocol selection.
//!
//! Identity is intentionally separate from runtime parser construction: a
//! local endpoint can speak an OpenAI-compatible protocol without being owned
//! by OpenAI.

/// Which model wire protocol handles a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelProtocol {
    Anthropic,
    OpenAi,
    Google,
    Ollama,
}

impl ModelProtocol {
    /// Stable name used by configuration and audit records.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAi => "openai",
            Self::Google => "google",
            Self::Ollama => "ollama",
        }
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

/// Which provider owns an endpoint for policy and audit purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    Unknown,
    Anthropic,
    OpenAi,
    Google,
    Ollama,
}

impl ProviderKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Anthropic => "anthropic",
            Self::OpenAi => "openai",
            Self::Google => "google",
            Self::Ollama => "ollama",
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

#[cfg(test)]
mod tests;
