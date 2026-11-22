//! Shared source and snapshot contract for read-only catalog discovery.
pub use capsem_config::catalogs::CatalogEntryMetadata;
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
