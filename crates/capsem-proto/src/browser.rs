//! Browser requests are ephemeral session state, never terminal escape codes
//! or ledger records: their query strings may contain login challenges.
use serde::{Deserialize, Serialize};

pub const MAX_BROWSER_URL_BYTES: usize = 8192;
pub const BROWSER_SOCKET_DIR: &str = "/run/capsem-browser";
pub const BROWSER_SOCKET: &str = "/run/capsem-browser/request.sock";

/// Redacted even in an unrecognized-port diagnostic.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BrowserUrl(pub String);

impl std::fmt::Debug for BrowserUrl {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("<browser URL>")
    }
}

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserRequest {
    pub id: u64,
    #[cfg_attr(feature = "schema", schema(max_length = 8192))]
    pub url: String,
}

impl BrowserRequest {
    pub fn validated_url(&self) -> Result<String, &'static str> {
        browser_url(&self.url)
    }
}

impl std::fmt::Debug for BrowserRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BrowserRequest")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// Only web navigation, with no credentials, control characters or implicit
/// scheme. The returned spelling is canonical and safe for a native opener.
pub fn browser_url(value: &str) -> Result<String, &'static str> {
    if value.is_empty() || value.len() > MAX_BROWSER_URL_BYTES || value.chars().any(char::is_control) {
        return Err("invalid browser URL length or control character");
    }
    let url = url::Url::parse(value).map_err(|_| "invalid browser URL")?;
    if !matches!(url.scheme(), "https" | "http")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("browser requests require HTTP(S) without credentials");
    }
    let value = url.to_string();
    if value.len() > MAX_BROWSER_URL_BYTES {
        return Err("browser URL too long after normalization");
    }
    Ok(value)
}

#[cfg(test)]
mod tests;
