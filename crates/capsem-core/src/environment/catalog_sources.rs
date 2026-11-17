//! Corporate source selection is independent of workload environment contents.
use capsem_assets::catalogs::{select_source, ValidatedCatalogSource};
use capsem_config::{
    catalogs::{CatalogKind, CatalogSources},
    SettingsFile,
};

/// Select against the separately loaded current corporate configuration.
/// Nothing in an archived or user-edited environment can override this source.
pub fn effective_catalog_source(
    manifest: &CatalogSources,
    corporate: &SettingsFile,
    kind: CatalogKind,
) -> Result<Option<ValidatedCatalogSource>, String> {
    select_source(
        manifest,
        corporate.catalogs.as_ref().unwrap_or(&CatalogSources::default()),
        kind,
    )
    .map_err(|_| "invalid effective environment catalog source".into())
}

pub(crate) fn validate_corporate_catalog_sources(corporate: &SettingsFile) -> Result<(), String> {
    for kind in CatalogKind::ALL {
        effective_catalog_source(&CatalogSources::default(), corporate, kind)?;
    }
    Ok(())
}
