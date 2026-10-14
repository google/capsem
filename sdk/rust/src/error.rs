/// Errors retain HTTP status and response bytes without exposing credentials.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("expected one VM named {name:?}, found {matches}")]
    VmLookup { name: String, matches: usize },
    #[error("invalid SDK input: {0}")]
    InvalidInput(&'static str),
    #[error("invalid gateway response: {0}")]
    InvalidResponse(&'static str),
    #[error("managed session cancelled")]
    Cancelled,
    #[error("managed session failed: {0}")]
    Managed(&'static str),
    #[error("managed cleanup failed: {source}; explicit close retry is available")]
    ManagedCleanup {
        source: Box<Error>,
        session: Box<crate::ManagedSession>,
    },
    #[error("gateway request failed: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("gateway returned HTTP {status}")]
    Http { status: u16, body: Vec<u8> },
    #[error("invalid gateway JSON: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// The broker's sanitized machine-readable refusal, when present. Unknown
    /// future codes or malformed/non-JSON responses retain their HTTP error.
    pub fn credential_code(&self) -> Option<crate::models::CredentialErrorCode> {
        #[derive(serde::Deserialize)]
        struct Envelope {
            code: crate::models::CredentialErrorCode,
        }
        let Self::Http { body, .. } = self else { return None };
        serde_json::from_slice::<Envelope>(body).ok().map(|value| value.code)
    }
}
