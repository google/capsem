/// Errors retain HTTP status and response bytes without exposing credentials.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("expected one VM named {name:?}, found {matches}")]
    VmLookup { name: String, matches: usize },
    #[error("invalid SDK input: {0}")]
    InvalidInput(&'static str),
    #[error("gateway request failed: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("gateway returned HTTP {status}")]
    Http { status: u16, body: Vec<u8> },
    #[error("invalid gateway JSON: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
