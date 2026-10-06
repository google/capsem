//! Long-lived installed cache ownership carries no registry credentials.

use anyhow::Result;

use super::{cache::BlobCache, CacheKey, CacheSnapshot, CacheUsage};

#[derive(Clone)]
pub struct ImageCache {
    pub(super) inner: BlobCache,
}

impl ImageCache {
    /// Bind the configured installed cache without accessing disk or network.
    pub fn installed() -> Result<Self> {
        Ok(Self {
            inner: BlobCache::installed()?,
        })
    }

    pub fn snapshot(&self, key: &CacheKey) -> Result<CacheSnapshot> {
        self.inner.snapshot(key)
    }

    /// Physical inventory is an explicit owner operation, not polling.
    pub async fn usage(&self) -> Result<CacheUsage> {
        self.inner.prepare().await?;
        self.inner.usage().await
    }

    #[cfg(test)]
    pub(super) fn at(root: &std::path::Path) -> Result<Self> {
        Ok(Self {
            inner: BlobCache::at(root)?,
        })
    }
}
