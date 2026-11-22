//! Shared source and snapshot contract for read-only catalog discovery.
pub use capsem_config::catalogs::CatalogEntryMetadata;
pub use capsem_proto::catalogs::{CatalogKind, CatalogSourceOrigin};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Default, Deserialize, IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in = Query)]
pub struct CatalogListQuery {
    /// Case-insensitive match on catalog entry metadata.
    pub search: Option<String>,
    /// Continue with the cursor for the same source and catalog digest.
    pub after: Option<String>,
    /// Number of matches per page, from 1 to 64. Defaults to 24.
    pub limit: Option<u16>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in = Query)]
pub struct CatalogEntryQuery {
    /// Exact published entry version.
    pub version: String,
    /// Source and digest token returned by the list.
    pub snapshot: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CatalogSnapshot {
    pub reference: String,
    pub digest: String,
    /// Supply this token when reading an entry from the snapshot.
    pub snapshot: String,
}

/// What the service learned on the last explicit catalog refresh.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CatalogAvailability {
    Unverified,
    Available,
    Stale,
    Unavailable,
}

/// Bounded diagnostic code. Registry responses and local configuration errors
/// may contain credentials or private details and are never exposed here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CatalogRefreshError {
    MissingSource,
    InvalidSource,
    PolicyUnavailable,
    RegistryUnavailable,
    VerificationFailed,
    LocalUnavailable,
    Unsupported,
}

/// Cached operator view for one effective catalog. Last success belongs to
/// the shown source only and remains informational when availability is stale.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CatalogStatus {
    pub kind: CatalogKind,
    pub availability: CatalogAvailability,
    #[schema(required = true)]
    pub reference: Option<String>,
    #[schema(required = true)]
    pub origin: Option<CatalogSourceOrigin>,
    /// Last verified OCI manifest digest.
    #[schema(required = true)]
    pub digest: Option<String>,
    /// Unix milliseconds of the last successful verified refresh.
    #[schema(required = true)]
    pub last_successful_refresh: Option<i64>,
    #[schema(required = true)]
    pub error: Option<CatalogRefreshError>,
}
