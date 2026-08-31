use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialProvider {
    Anthropic,
    Google,
    OpenAi,
    Github,
    Mcp,
}

impl CredentialProvider {
    pub fn all() -> &'static [Self] {
        &[Self::Anthropic, Self::Google, Self::OpenAi, Self::Github, Self::Mcp]
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::Google => "google",
            Self::OpenAi => "openai",
            Self::Github => "github",
            Self::Mcp => "mcp",
        }
    }
}

pub(crate) fn credential_provider_from_str(provider: &str) -> Option<CredentialProvider> {
    match provider {
        "anthropic" => Some(CredentialProvider::Anthropic),
        "google" => Some(CredentialProvider::Google),
        "openai" => Some(CredentialProvider::OpenAi),
        "github" => Some(CredentialProvider::Github),
        "mcp" => Some(CredentialProvider::Mcp),
        _ => None,
    }
}
