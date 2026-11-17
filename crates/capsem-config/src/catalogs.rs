//! Shared catalog source contracts live below config and OCI consumers.
pub use capsem_proto::catalogs::{
    CatalogKind, CatalogSourceDescriptor, CatalogSourceOrigin, CatalogSources, SelectedCatalogSource,
    ENVIRONMENT_CATALOG_SCHEMA_VERSION,
};
mod content;
pub use content::{
    CatalogCompatibility, CatalogDocument, CatalogEntry, CatalogEntryMetadata, CatalogEntryStatus,
    CATALOG_RUNTIME_CONTRACT,
};

#[cfg(test)]
mod tests;
