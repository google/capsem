//! Shared catalog source contracts live below config and OCI consumers.
pub use capsem_proto::catalogs::{
    CatalogKind, CatalogSourceDescriptor, CatalogSourceOrigin, CatalogSources, SelectedCatalogSource,
    ENVIRONMENT_CATALOG_SCHEMA_VERSION,
};
mod collections;
mod content;
mod rules;
mod skills;
pub use content::{
    CatalogCompatibility, CatalogDocument, CatalogEntry, CatalogEntryMetadata, CatalogEntryStatus,
    CATALOG_RUNTIME_CONTRACT,
};
pub use rules::{CatalogRuleParameter, ResolvedCatalogRule, RuleCatalogDefinition, RuleCatalogEntry};
pub use skills::{SkillCatalogEntry, SkillFile, SkillPayload};

#[cfg(test)]
mod tests;
