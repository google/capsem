//! Complete materialization proof is separate from passive readiness state.

use super::*;

impl Puller {
    /// Reconstruct every required image and bound filesystem locally, using
    /// the normal platform, manifest, size and digest validators. A decoded
    /// receipt cannot omit dependencies or supply different filesystem facts.
    ///
    /// This is a bounded verification operation, not a polling API. Success
    /// records facts verified during this call, never ongoing readiness or
    /// permission to execute; external mutation invalidation is still needed.
    pub async fn reconcile_cached_receipt(
        &self,
        key: &super::super::CacheKey,
        parent: &Path,
    ) -> Result<Option<CacheReceipt>> {
        tokio::time::timeout(PULL_TIMEOUT, self.reconcile_inner(key, parent))
            .await
            .context("cached receipt reconciliation exceeded five minutes")?
    }

    async fn reconcile_inner(&self, key: &super::super::CacheKey, parent: &Path) -> Result<Option<CacheReceipt>> {
        let Some(receipt) = self.cached_receipt(key).await? else {
            return Ok(None);
        };
        ensure!(
            receipt.identity().architecture() == self.architecture
                && receipt.identity().runtime_contract() == RUNTIME_CONTRACT,
            "cached receipt requires a different native runtime"
        );
        let image = self.pull_inner(receipt.origin(), parent, true).await?;
        ensure!(
            receipt.matches_image(&image.materialization),
            "cache receipt does not match the complete image graph"
        );
        if let Some(expected) = receipt.root() {
            let filesystem = self
                .fetch_rootfs_inner(&expected.origin, receipt.native_digest(), parent, true)
                .await?;
            ensure!(
                filesystem.receipt == *expected,
                "cache receipt does not match the bound filesystem graph"
            );
        }
        let current = self
            .cached_receipt(key)
            .await?
            .context("receipt disappeared during verification")?;
        ensure!(
            current.generation()? == receipt.generation()?,
            "receipt changed during verification"
        );
        Ok(Some(receipt))
    }
}

#[cfg(test)]
mod tests;
