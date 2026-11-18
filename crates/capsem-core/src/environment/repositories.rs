//! Host-owned Git metadata discovery. Resolution is not permission to clone,
//! fetch credentials, execute preparation, or change a sandbox's network policy.
mod advertisement;
mod https;
pub use https::{RepositoryAdmission, RepositoryHttpsResolver, RepositoryNetworkAuthority};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepositoryResolutionError {
    InvalidIntent,
    Refused,
    Unavailable,
    Deadline,
    TooLarge,
    InvalidAdvertisement,
    UnknownReference,
    AmbiguousReference,
}
impl std::fmt::Display for RepositoryResolutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidIntent => "invalid HTTPS repository selection",
            Self::Refused => "repository network admission refused",
            Self::Unavailable => "repository discovery unavailable",
            Self::Deadline => "repository discovery deadline exceeded",
            Self::TooLarge => "repository advertisement exceeds its byte limit",
            Self::InvalidAdvertisement => "invalid repository advertisement",
            Self::UnknownReference => "repository reference was not advertised",
            Self::AmbiguousReference => "repository branch and tag selection is ambiguous",
        })
    }
}
impl std::error::Error for RepositoryResolutionError {}
