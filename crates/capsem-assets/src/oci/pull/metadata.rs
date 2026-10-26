//! Shared native-platform manifest selection for image and inventory reads.
use super::*;

impl Puller {
    pub(super) async fn platform_manifest(
        &self,
        reference: &Reference,
        first: &[u8],
        token: Option<&str>,
        cache_only: bool,
    ) -> Result<Vec<u8>> {
        Ok(match serde_json::from_slice::<OciManifest>(first)? {
            OciManifest::Image(_) => first.to_vec(),
            OciManifest::ImageIndex(index) => {
                ensure!(index.schema_version == 2, "unsupported OCI index schema");
                let selected = index
                    .manifests
                    .iter()
                    .find(|entry| {
                        entry.platform.as_ref().is_some_and(|p| {
                            p.os == "linux".into()
                                && p.architecture == self.architecture.as_str().into()
                                && p.variant
                                    .as_deref()
                                    .is_none_or(|v| self.architecture == "arm64" && v == "v8")
                                && p.os_features.as_ref().is_none_or(Vec::is_empty)
                                && p.features.as_ref().is_none_or(Vec::is_empty)
                        })
                    })
                    .context("registry has no compatible native Linux image")?;
                digest_hex(&selected.digest)?;
                ensure!(
                    selected.size > 0 && selected.size <= METADATA_LIMIT as i64,
                    "manifest size exceeds limit"
                );
                let pinned = Reference::with_digest(
                    reference.registry().into(),
                    reference.repository().into(),
                    selected.digest.clone(),
                );
                let bytes = if cache_only {
                    self.cached_manifest(&pinned).await?
                } else {
                    self.manifest(&pinned, token).await?
                };
                ensure!(bytes.len() == selected.size as usize, "manifest size mismatch");
                bytes
            }
        })
    }
}
