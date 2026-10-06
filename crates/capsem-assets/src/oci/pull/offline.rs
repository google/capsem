//! Local-only materialization shares the online transport's validation path.

use super::*;

impl Puller {
    /// Reuse a verified published filesystem locally, preserving its subject
    /// binding and immutable cache-generation protection.
    pub async fn fetch_rootfs_cached(
        &self,
        reference: &str,
        subject: &ContentDigest,
        parent: &Path,
    ) -> Result<RootfsLayout> {
        ensure!(
            self.cache.is_some(),
            "cache-only materialization requires an image cache"
        );
        tokio::time::timeout(PULL_TIMEOUT, self.fetch_rootfs_inner(reference, subject, parent, true))
            .await
            .context("cached rootfs materialization exceeded five minutes")?
    }

    /// Reconstruct a pinned native layout using only verified local bytes.
    /// The caller must independently admit this source under current policy;
    /// cached content never grants execution authority.
    pub async fn pull_cached(&self, reference: &str, parent: &Path) -> Result<ImageLayout> {
        ensure!(
            image_reference(reference)?.digest().is_some(),
            "cache-only images must be pinned by digest"
        );
        ensure!(
            self.cache.is_some(),
            "cache-only materialization requires an image cache"
        );
        tokio::time::timeout(PULL_TIMEOUT, self.pull_inner(reference, parent, true))
            .await
            .context("cached OCI materialization exceeded five minutes")?
    }

    pub(super) async fn cached_manifest(&self, reference: &Reference) -> Result<Vec<u8>> {
        self.cache
            .as_ref()
            .context("cache-only materialization requires an image cache")?
            .for_repository(reference)
            .read_metadata(
                reference.digest().context("cache-only manifest must be pinned")?,
                METADATA_LIMIT,
            )
            .await?
            .context("required OCI manifest is missing or corrupt in the local cache")
    }
}

#[cfg(test)]
mod tests;
