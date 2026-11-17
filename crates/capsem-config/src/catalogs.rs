//! Shared catalog source contracts live below config and OCI consumers.
pub use capsem_proto::catalogs::{
    CatalogKind, CatalogSourceDescriptor, CatalogSourceOrigin, CatalogSources, SelectedCatalogSource,
    ENVIRONMENT_CATALOG_SCHEMA_VERSION,
};

#[cfg(test)]
mod tests;
