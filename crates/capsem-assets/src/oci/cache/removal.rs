//! Read-only removal planning runs under the installed owner's existing lock.

use std::collections::{BTreeMap, BTreeSet};

use super::super::{
    image_reference,
    receipts::{BlobKind, CacheReceipt},
    removal::{BlobState, FileState},
    CacheKey, RemovalPreview,
};
use super::*;

impl BlobCache {
    pub(in super::super) async fn preview_removal(&self, key: &CacheKey) -> Result<RemovalPreview> {
        let lease = lock_with(self.mutation_lock(), LockAccess::Existing, LockMode::Exclusive).await?;
        let (cache, key) = (self.clone(), key.clone());
        tokio::task::spawn_blocking(move || {
            let _lease = lease;
            let root = ContainedDir::open_root(&cache.root)?;
            root.validate_private()?;
            let directory = root.walk(&cache.policy.entry_root)?;
            preview(&cache, &root, &directory, &key)
        })
        .await?
    }
}

fn preview(cache: &BlobCache, root: &ContainedDir, directory: &ContainedDir, key: &CacheKey) -> Result<RemovalPreview> {
    let receipt = receipts::read(directory, key)?.context("cache receipt is missing")?;
    let mut references = Vec::new();
    let mut retained = BTreeSet::new();
    for entry in directory.entries()? {
        let name = entry.name.to_string_lossy();
        let Some(candidate) = name.strip_prefix("receipt-") else {
            continue;
        };
        let candidate = CacheKey::parse(candidate).context("unrecognized receipt prevents reference-safe removal")?;
        if candidate == *key {
            continue;
        }
        let other = receipts::read(directory, &candidate)?.context("receipt changed during removal preview")?;
        retained.extend(names(cache, &other)?.into_keys());
        references.push((candidate.as_str().to_owned(), other.generation()?));
    }
    references.sort();
    let blobs = names(cache, &receipt)?
        .into_iter()
        .map(|(name, kind)| {
            let file = match directory.open_file(name.as_ref(), ContainedOpenOptions::read_only()) {
                Ok(file) => Some(FileState::from_metadata(&file.metadata()?)?),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error.into()),
            };
            let shared = retained.contains(&name);
            Ok(BlobState {
                name,
                kind,
                file,
                shared,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let control = directory.open_file(
        format!("receipt-{}", key.as_str()).as_ref(),
        ContainedOpenOptions::read_only(),
    )?;
    RemovalPreview::new(
        key.clone(),
        receipt.generation()?,
        references,
        FileState::from_metadata(&control.metadata()?)?,
        blobs,
        materializing(root)?,
    )
}

fn names(cache: &BlobCache, receipt: &CacheReceipt) -> Result<BTreeMap<String, BlobKind>> {
    let mut names = BTreeMap::new();
    for (origin, blobs) in std::iter::once((receipt.origin(), receipt.blobs()))
        .chain(receipt.root().map(|root| (root.origin.as_str(), root.blobs.as_slice())))
    {
        let scoped = cache.for_repository(&image_reference(origin)?);
        for blob in blobs {
            let name = scoped.entry_name(&blob.digest)?;
            let name = if blob.kind == BlobKind::ImmutableRoot {
                format!("immutable-{name}")
            } else {
                name
            };
            names.insert(name, blob.kind);
        }
    }
    Ok(names)
}

fn materializing(root: &ContainedDir) -> Result<bool> {
    let directory = root.descend("locks".as_ref())?;
    match try_acquire_existing(&directory.path().join("materialization.lock"), LockMode::Exclusive) {
        Ok(LockAttempt::Contended) => return Ok(true),
        Ok(LockAttempt::Acquired(_lease)) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    for entry in directory.entries()? {
        let name = entry.name.to_string_lossy();
        let Some(key) = name.strip_prefix("materialize-") else {
            continue;
        };
        CacheKey::parse(key.strip_suffix(".lock").context("invalid materialization lock name")?)?;
        // Existing-only acquisition neither creates nor chmods controls.
        match try_acquire_existing(&directory.path().join(&entry.name), LockMode::Exclusive)? {
            LockAttempt::Contended => return Ok(true),
            LockAttempt::Acquired(_lease) => {}
        }
    }
    Ok(false)
}
