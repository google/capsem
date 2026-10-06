//! Receipt operations share the blob owner's mutation boundary and capacity.

use super::super::receipts::{BlobKind, BlobRef, CacheReceipt, RootReceipt};
use super::super::{image_reference, CacheKey, METADATA_LIMIT};
use super::*;

impl BlobCache {
    pub(in super::super) async fn read_receipt(&self, key: &CacheKey) -> Result<Option<CacheReceipt>> {
        let lease = read_lock(self.mutation_lock()).await?;
        let (cache, key) = (self.clone(), key.clone());
        tokio::task::spawn_blocking(move || {
            let _lease = lease;
            let directory = ContainedDir::open_root(&cache.root)?.walk(&cache.policy.entry_root)?;
            read(&directory, &key)
        })
        .await?
    }

    pub(in super::super) async fn publish_receipt(&self, receipt: CacheReceipt) -> Result<()> {
        let lease = self.mutation_lease().await?;
        let cache = self.clone();
        tokio::task::spawn_blocking(move || {
            let _lease = lease;
            let directory = ContainedDir::open_root(&cache.root)?.walk(&cache.policy.entry_root)?;
            publish(&cache, &directory, receipt)
        })
        .await?
    }

    pub(in super::super) async fn bind_root_receipt(&self, key: &CacheKey, root: RootReceipt) -> Result<()> {
        let lease = self.mutation_lease().await?;
        let (cache, key) = (self.clone(), key.clone());
        tokio::task::spawn_blocking(move || {
            let _lease = lease;
            let directory = ContainedDir::open_root(&cache.root)?.walk(&cache.policy.entry_root)?;
            let receipt = read(&directory, &key)?.context("image receipt is missing")?;
            if receipt.root() == Some(&root) {
                return Ok(());
            }
            publish(&cache, &directory, receipt.with_root(root)?)
        })
        .await?
    }
}

pub(super) fn read(directory: &ContainedDir, key: &CacheKey) -> Result<Option<CacheReceipt>> {
    let file = match directory.open_file(
        format!("receipt-{}", key.as_str()).as_ref(),
        ContainedOpenOptions::read_only(),
    ) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        file.metadata()?.len() <= METADATA_LIMIT as u64,
        "cache receipt exceeds metadata limit"
    );
    let mut bytes = Vec::new();
    file.take((METADATA_LIMIT + 1) as u64).read_to_end(&mut bytes)?;
    Ok(Some(CacheReceipt::decode(&bytes, key)?))
}

fn publish(cache: &BlobCache, directory: &ContainedDir, receipt: CacheReceipt) -> Result<()> {
    let bytes = receipt.encode()?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".partial-receipt-")
        .tempfile_in(directory.path())?;
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    let metadata = temporary.as_file().metadata()?;
    cache.prune_keeping(directory, Some((metadata.dev(), metadata.ino())))?;
    verify(cache, directory, receipt.origin(), receipt.blobs())?;
    if let Some(root) = receipt.root() {
        verify(cache, directory, &root.origin, &root.blobs)?;
    }
    let name = format!("receipt-{}", receipt.key().as_str());
    temporary.persist(directory.path().join(&name))?;
    if let Err(error) = directory
        .sync()
        .map_err(anyhow::Error::from)
        .and_then(|()| cache.check_capacity(&ContainedDir::open_root(&cache.root)?))
    {
        directory.remove_non_directory(name.as_ref())?;
        directory.sync()?;
        return Err(error);
    }
    Ok(())
}

pub(super) fn verify(cache: &BlobCache, directory: &ContainedDir, origin: &str, blobs: &[BlobRef]) -> Result<()> {
    let scoped = cache.for_repository(&image_reference(origin)?);
    for blob in blobs {
        let name = scoped.entry_name(&blob.digest)?;
        let name = if blob.kind == BlobKind::ImmutableRoot {
            format!("immutable-{name}")
        } else {
            name
        };
        let file = directory.open_file(name.as_ref(), ContainedOpenOptions::read_only())?;
        ensure!(
            blob.kind != BlobKind::ImmutableRoot || file.metadata()?.mode() & 0o777 == 0o444,
            "receipt filesystem must be read-only"
        );
        ensure!(
            verified_bytes(file, &blob.digest, blob.size, |_| Ok(()))?,
            "required receipt blob is missing or corrupt"
        );
    }
    Ok(())
}
