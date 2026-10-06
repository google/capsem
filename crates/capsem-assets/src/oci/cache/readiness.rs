//! Readiness proof and mutation epochs belong to the blob owner.

use std::time::UNIX_EPOCH;

use capsem_foundation::unix::change_watch::ChangeWatch;

use super::super::{
    receipts::{BlobKind, BlobRef, CacheReceipt},
    CacheKey, CacheSnapshot, CacheState,
};
use super::*;

pub(in super::super) struct Verification {
    tracking: Arc<Mutex<super::super::readiness::Tracking>>,
    finished: bool,
}

impl Verification {
    pub(in super::super) fn finish(&mut self) {
        self.finished = true;
    }
}

impl Drop for Verification {
    fn drop(&mut self) {
        if !self.finished {
            if let Ok(mut tracking) = self.tracking.lock() {
                // Cancellation obsoletes a proof still running on the blocking
                // pool. Poisoning/epoch exhaustion also cannot retain entries.
                let _ = tracking.invalidate();
            }
        }
    }
}

impl BlobCache {
    pub(in super::super) fn verification(&self) -> Result<Verification> {
        self.tracking
            .lock()
            .map_err(|_| anyhow::anyhow!("OCI tracking lock poisoned"))?
            .invalidate()?;
        Ok(Verification {
            tracking: self.tracking.clone(),
            finished: false,
        })
    }

    /// Invalidate before any owned filesystem mutation can become visible.
    pub(in super::super) async fn mutation_lease(&self) -> Result<FileLock> {
        let lease = lock(self.mutation_lock()).await?;
        self.tracking
            .lock()
            .map_err(|_| anyhow::anyhow!("OCI tracking lock poisoned"))?
            .invalidate()?;
        Ok(lease)
    }

    pub(in super::super) fn snapshot(&self, key: &CacheKey) -> Result<CacheSnapshot> {
        self.tracking
            .lock()
            .map_err(|_| anyhow::anyhow!("OCI tracking lock poisoned"))?
            .snapshot(key)
    }

    /// Reobserve negative facts after installing inode/name watches. A prior
    /// error alone cannot publish a fresh observation: external repair may
    /// already have happened while the failed materialization was unwinding.
    pub(in super::super) async fn incomplete(&self, key: &CacheKey) -> Result<CacheSnapshot> {
        let lease = read_lock(self.mutation_lock()).await?;
        let epoch = self
            .tracking
            .lock()
            .map_err(|_| anyhow::anyhow!("OCI tracking lock poisoned"))?
            .epoch();
        let (cache, key) = (self.clone(), key.clone());
        tokio::task::spawn_blocking(move || -> Result<CacheSnapshot> {
            let _lease = lease;
            let mut watch = ChangeWatch::new()?;
            let directory = watched_directory(&cache, &mut watch)?;
            let name = format!("receipt-{}", key.as_str());
            attach_existing(&directory, &mut watch, &name)?;
            let state = match receipts::read(&directory, &key) {
                Ok(None) => CacheState::Missing,
                Err(_) => CacheState::Partial,
                Ok(Some(receipt)) => {
                    attach(&cache, &directory, &mut watch, receipt.origin(), receipt.blobs(), false)?;
                    if let Some(root) = receipt.root() {
                        attach(&cache, &directory, &mut watch, &root.origin, &root.blobs, false)?;
                    }
                    let valid =
                        receipts::verify(&cache, &directory, receipt.origin(), receipt.blobs()).and_then(|()| {
                            if let Some(root) = receipt.root() {
                                receipts::verify(&cache, &directory, &root.origin, &root.blobs)?;
                            }
                            Ok(())
                        });
                    if valid.is_err() {
                        CacheState::Partial
                    } else {
                        CacheState::Unknown
                    }
                }
            };
            ensure!(!watch.changed()?, "cache changed during incomplete observation");
            let mut tracking = cache
                .tracking
                .lock()
                .map_err(|_| anyhow::anyhow!("OCI tracking lock poisoned"))?;
            ensure!(
                tracking.epoch() == epoch && epoch != u64::MAX,
                "incomplete observation was invalidated"
            );
            tracking.observed(key.clone(), state, None, Some(watch));
            tracking.snapshot(&key)
        })
        .await?
    }

    /// The full graph was independently reconciled. Watch every required
    /// inode before a readonly rehash, then publish under the local lease.
    pub(in super::super) async fn verify_snapshot(&self, receipt: CacheReceipt) -> Result<CacheSnapshot> {
        let lease = read_lock(self.mutation_lock()).await?;
        let epoch = self
            .tracking
            .lock()
            .map_err(|_| anyhow::anyhow!("OCI tracking lock poisoned"))?
            .epoch();
        let cache = self.clone();
        tokio::task::spawn_blocking(move || -> Result<CacheSnapshot> {
            let _lease = lease;
            let mut watch = ChangeWatch::new()?;
            let directory = watched_directory(&cache, &mut watch)?;
            let key = receipt.key();
            let control = directory.open_file(
                format!("receipt-{}", key.as_str()).as_ref(),
                ContainedOpenOptions::read_only(),
            )?;
            watch.add(control.as_fd())?;
            attach(&cache, &directory, &mut watch, receipt.origin(), receipt.blobs(), true)?;
            if let Some(filesystem) = receipt.root() {
                attach(
                    &cache,
                    &directory,
                    &mut watch,
                    &filesystem.origin,
                    &filesystem.blobs,
                    true,
                )?;
            }
            let current = receipts::read(&directory, &key)?.context("receipt disappeared before proof")?;
            ensure!(
                current.generation()? == receipt.generation()?,
                "receipt changed before proof"
            );
            receipts::verify(&cache, &directory, receipt.origin(), receipt.blobs())?;
            if let Some(filesystem) = receipt.root() {
                receipts::verify(&cache, &directory, &filesystem.origin, &filesystem.blobs)?;
            }
            ensure!(!watch.changed()?, "cache changed during readiness verification");
            let verified_at = u64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos())?;
            let mut tracking = cache
                .tracking
                .lock()
                .map_err(|_| anyhow::anyhow!("OCI tracking lock poisoned"))?;
            ensure!(
                tracking.epoch() == epoch && epoch != u64::MAX,
                "readiness proof was cancelled or invalidated"
            );
            tracking.observed(key.clone(), CacheState::Ready, Some(verified_at), Some(watch));
            tracking.snapshot(&key)
        })
        .await?
    }
}

fn watched_directory(cache: &BlobCache, watch: &mut ChangeWatch) -> Result<ContainedDir> {
    let mut directory = ContainedDir::open_root(&cache.root)?;
    watch.add(directory.as_fd())?;
    for component in cache.policy.entry_root.components() {
        let std::path::Component::Normal(name) = component else {
            anyhow::bail!("cache entry root must contain plain relative names");
        };
        directory = directory.descend(name)?;
        watch.add(directory.as_fd())?;
    }
    Ok(directory)
}

fn attach_existing(directory: &ContainedDir, watch: &mut ChangeWatch, name: &str) -> Result<()> {
    if directory.entry_kind(name.as_ref())? == Some(EntryKind::File) {
        let file = directory.open_file(name.as_ref(), ContainedOpenOptions::read_only())?;
        watch.add(file.as_fd())?;
    }
    Ok(())
}

fn attach(
    cache: &BlobCache,
    directory: &ContainedDir,
    watch: &mut ChangeWatch,
    origin: &str,
    blobs: &[BlobRef],
    required: bool,
) -> Result<()> {
    let scoped = cache.for_repository(&super::super::image_reference(origin)?);
    for blob in blobs {
        let name = scoped.entry_name(&blob.digest)?;
        let name = if blob.kind == BlobKind::ImmutableRoot {
            format!("immutable-{name}")
        } else {
            name
        };
        if required {
            let file = directory.open_file(name.as_ref(), ContainedOpenOptions::read_only())?;
            watch.add(file.as_fd())?;
        } else {
            attach_existing(directory, watch, &name)?;
        }
    }
    Ok(())
}
