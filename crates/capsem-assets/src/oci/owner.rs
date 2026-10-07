//! Long-lived installed cache ownership carries no registry credentials.

use anyhow::Result;
use std::sync::Arc;

use super::{cache::BlobCache, CacheInventory, CacheKey, CacheSnapshot, CacheUsage};

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

    /// Explicit descriptor-owned inventory, never an idle polling operation.
    pub async fn inventory(&self) -> Result<CacheInventory> {
        self.inner.prepare().await?;
        self.inner.inventory().await
    }

    /// A cheap memory/kernel-notification read; absence means unobserved or
    /// invalidated. It never traverses disk or grants deletion authority.
    pub fn inventory_snapshot(&self) -> Result<Option<Arc<CacheInventory>>> {
        self.inner.inventory_snapshot()
    }

    /// Explicit bounded collection. Directory bindings and regular-file
    /// inodes are watched before observing; changes refuse publication.
    pub async fn refresh_inventory(&self, maximum_entries: usize) -> Result<()> {
        anyhow::ensure!(maximum_entries > 0, "inventory entry budget must be positive");
        self.inner.prepare().await?;
        self.inner.refresh_inventory(maximum_entries).await
    }

    #[cfg(test)]
    pub(super) fn at(root: &std::path::Path) -> Result<Self> {
        Ok(Self {
            inner: BlobCache::at(root)?,
        })
    }
}
