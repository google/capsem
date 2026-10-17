//! Owner readiness separates verification work from polling.

use super::super::{CacheKey, CacheSnapshot};
use super::*;

impl Puller {
    /// Cheap owner observation: memory plus bounded nonblocking kernel reads.
    /// No cache path traversal, file bytes, hashes or registry requests.
    pub fn cache_snapshot(&self, key: &CacheKey) -> Result<CacheSnapshot> {
        self.cache
            .as_ref()
            .context("snapshot requires an image cache")?
            .snapshot(key)
    }

    pub async fn reconcile_cache(&self, key: &CacheKey, parent: &Path) -> Result<CacheSnapshot> {
        let cache = self.cache.as_ref().context("readiness requires an image cache")?;
        let mut verification = cache.verification(key)?;
        let result = tokio::time::timeout(PULL_TIMEOUT, async {
            match self.reconcile_cached_receipt(key, parent).await {
                Ok(Some(receipt)) => match cache.verify_snapshot(receipt).await {
                    Ok(snapshot) => Ok(snapshot),
                    Err(error) => {
                        cache.incomplete(key, &self.architecture).await?;
                        Err(error)
                    }
                },
                Ok(None) => cache.incomplete(key, &self.architecture).await,
                Err(error) => {
                    cache.incomplete(key, &self.architecture).await?;
                    Err(error)
                }
            }
        })
        .await;
        if result.is_ok() {
            verification.finish();
        }
        result.context("cache readiness reconciliation exceeded five minutes")?
    }
}

#[cfg(test)]
mod tests;
