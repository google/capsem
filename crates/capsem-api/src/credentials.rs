//! Explicit host credential injection. Responses never expose material.
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CredentialInjectProvider {
    Anthropic,
    Google,
    Openai,
    Github,
    Mcp,
}

impl CredentialInjectProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::Google => "google",
            Self::Openai => "openai",
            Self::Github => "github",
            Self::Mcp => "mcp",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CredentialStorage {
    #[default]
    File,
    Memory,
}

#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CredentialInjectRequest {
    pub provider: CredentialInjectProvider,
    #[schema(write_only = true)]
    pub value: String,
    #[serde(default)]
    pub storage: CredentialStorage,
}

impl std::fmt::Debug for CredentialInjectRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialInjectRequest")
            .field("provider", &self.provider)
            .field("storage", &self.storage)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CredentialInjectResponse {
    pub credential_ref: String,
    pub storage: CredentialStorage,
}

#[cfg(test)]
mod tests;
