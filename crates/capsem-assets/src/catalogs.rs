//! Verified environment catalogs reuse the existing OCI source/cache owner.
use crate::oci::ImageReference;
use anyhow::{anyhow, Result};
use capsem_proto::catalogs::{CatalogKind, CatalogSourceOrigin, SelectedCatalogSource};
use oci_client::Reference;

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
