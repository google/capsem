//! Verified environment catalogs reuse the existing OCI source/cache owner.
use crate::oci::ImageReference;
use anyhow::{anyhow, Result};
use capsem_proto::catalogs::{CatalogKind, CatalogSourceOrigin, CatalogSources, SelectedCatalogSource};
use oci_client::Reference;
use serde::Deserialize;

/// Read metadata from the selected public or legacy manifest without requiring
/// a VM runtime. JSON is the external release-manifest format; unrelated release
/// fields are skipped rather than copied into another release document.
pub fn sources_from_manifest(content: &str) -> Result<CatalogSources> {
    anyhow::ensure!(content.trim_start().starts_with('{'), "manifest must be a JSON object");
    #[derive(Deserialize)]
    struct Metadata {
        #[serde(default)]
        catalogs: Option<CatalogSources>,
    }
    let metadata: Metadata =
        serde_json::from_str(content).map_err(|_| anyhow!("invalid manifest catalog source metadata"))?;
    Ok(metadata.catalogs.unwrap_or_default())
}

/// Validate only the effective source. An invalid corporate selection remains
/// an error; it must never trigger validation or use of the manifest fallback.
pub fn select_source(
    manifest: &CatalogSources,
    corporate: &CatalogSources,
    kind: CatalogKind,
) -> Result<Option<ValidatedCatalogSource>> {
    manifest
        .effective_source(corporate, kind)
        .map(|selected| ValidatedCatalogSource::new(kind, selected))
        .transpose()
}

/// Format-validated source, not an assertion of catalog authenticity. Preserve
/// the requested tag for resolution and the normalized identity for admission.
#[derive(Clone, Debug)]
pub struct ValidatedCatalogSource {
    kind: CatalogKind,
    origin: CatalogSourceOrigin,
    schema_version: u16,
    reference: Reference,
    identity: ImageReference,
}
impl ValidatedCatalogSource {
    pub fn new(kind: CatalogKind, selected: SelectedCatalogSource<'_>) -> Result<Self> {
        selected.source.validate().map_err(anyhow::Error::msg)?;
        let reference = crate::oci::image_reference(&selected.source.reference)
            .map_err(|_| anyhow!("invalid environment catalog OCI reference"))?;
        let identity =
            ImageReference::try_from(&reference).map_err(|_| anyhow!("invalid environment catalog OCI identity"))?;
        Ok(Self {
            kind,
            origin: selected.origin,
            schema_version: selected.source.schema_version,
            reference,
            identity,
        })
    }
    pub fn kind(&self) -> CatalogKind {
        self.kind
    }
    pub fn origin(&self) -> CatalogSourceOrigin {
        self.origin
    }
    pub fn schema_version(&self) -> u16 {
        self.schema_version
    }
    pub fn reference(&self) -> &Reference {
        &self.reference
    }
    pub fn identity(&self) -> &ImageReference {
        &self.identity
    }
}

#[cfg(test)]
mod tests;
