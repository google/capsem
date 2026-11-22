//! Verified environment catalogs reuse the existing OCI source/cache owner.
use crate::oci::ImageReference;
use anyhow::{anyhow, Result};
use capsem_proto::catalogs::{CatalogKind, CatalogSourceOrigin, CatalogSources, SelectedCatalogSource};
use oci_client::Reference;
use serde::Deserialize;

/// The independently published single JSON layer for each catalog kind.
pub const fn catalog_media_type(kind: CatalogKind) -> &'static str {
    match kind {
        CatalogKind::Skills => "application/vnd.capsem.skills.catalog.v1+json",
        CatalogKind::Rules => "application/vnd.capsem.rules.catalog.v1+json",
        CatalogKind::Mcp => "application/vnd.capsem.mcp.catalog.v1+json",
    }
}

/// OCI-verified transport bytes. The chosen source supplies the expected MCP
/// digest. Schema, revocation and environment policy admission apply separately.
pub struct CatalogArtifact {
    kind: CatalogKind,
    digest: crate::oci::Digest,
    bytes: Vec<u8>,
}
impl std::fmt::Debug for CatalogArtifact {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CatalogArtifact")
            .field("kind", &self.kind)
            .field("digest", &self.digest)
            .field("byte_count", &self.bytes.len())
            .finish_non_exhaustive()
    }
}
impl CatalogArtifact {
    pub(crate) fn new(kind: CatalogKind, digest: crate::oci::Digest, bytes: Vec<u8>) -> Self {
        Self { kind, digest, bytes }
    }
    pub fn kind(&self) -> CatalogKind {
        self.kind
    }
    pub fn digest(&self) -> &crate::oci::Digest {
        &self.digest
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

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

/// Validated source from the effective manifest or corporate policy.
/// Preserve tags where allowed and normalize the identity for digest admission.
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
        // A catalog that describes MCP launches must bind its content to the
        // effective authority's digest before the OCI client contacts a registry.
        anyhow::ensure!(
            kind != CatalogKind::Mcp || identity.digest().is_some(),
            "MCP catalog source requires a SHA-256 OCI manifest digest"
        );
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
