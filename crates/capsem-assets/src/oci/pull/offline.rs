//! Local-only materialization shares the online transport's validation path.

use super::*;

impl Puller {
    /// Traverse owner-managed disk usage off the async worker. This is a
    /// reconciliation operation, not a cheap polling snapshot or readiness.
    pub async fn cache_usage(&self) -> Result<super::super::CacheUsage> {
        let cache = self.cache.as_ref().context("inventory requires an image cache")?;
        cache.prepare().await?;
        cache.usage().await
    }

    /// Read durable verification metadata. It must be reconciled before a
    /// caller reports readiness, and it never supplies execution authority.
    pub async fn cached_receipt(&self, key: &super::super::CacheKey) -> Result<Option<CacheReceipt>> {
        let cache = self.cache.as_ref().context("receipt lookup requires an image cache")?;
        cache.prepare().await?;
        cache.read_receipt(key).await
    }

    /// Bind a verified published filesystem to its original image receipt.
    pub async fn retain_cached_root(&self, key: &super::super::CacheKey, root: &RootfsLayout) -> Result<()> {
        let cache = self.cache.as_ref().context("root retention requires an image cache")?;
        cache.bind_root_receipt(key, root.receipt.clone()).await
    }
    /// Prefer verified local bytes for an admitted immutable image. Moving
    /// tags still resolve online; a miss uses the normal registry transport.
    pub async fn pull_prefer_cached(&self, reference: &str, parent: &Path) -> Result<ImageLayout> {
        let pinned = image_reference(reference)?.digest().is_some();
        tokio::time::timeout(PULL_TIMEOUT, async {
            if pinned && self.cache.is_some() {
                if let Ok(layout) = self.pull_inner(reference, parent, true).await {
                    return Ok(layout);
                }
            }
            self.pull_inner(reference, parent, false).await
        })
        .await
        .context("OCI materialization exceeded five minutes")?
    }

    /// Use the protected local generation if available, otherwise fetch and
    /// verify the published artifact. Both attempts share one operation bound.
    pub async fn fetch_rootfs_prefer_cached(
        &self,
        reference: &str,
        subject: &ContentDigest,
        parent: &Path,
    ) -> Result<RootfsLayout> {
        tokio::time::timeout(PULL_TIMEOUT, async {
            if self.cache.is_some() {
                if let Ok(layout) = self.fetch_rootfs_inner(reference, subject, parent, true).await {
                    return Ok(layout);
                }
            }
            self.fetch_rootfs_inner(reference, subject, parent, false).await
        })
        .await
        .context("rootfs materialization exceeded five minutes")?
    }

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
