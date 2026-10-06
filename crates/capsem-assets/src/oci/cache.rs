//! Verified OCI blob retention. Policy is shared with the repository cache owner.

use std::{
    collections::BTreeMap,
    fs::{File, FileTimes},
    io::{Read, Write},
    os::{fd::AsFd, unix::fs::MetadataExt},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use anyhow::{ensure, Context, Result};
use capsem_foundation::{
    paths,
    poll::{poll_until, PollOpts},
    unix::{
        contained::{ContainedDir, ContainedOpenOptions, EntryKind},
        fs::ensure_private_dir,
        lock::{try_acquire, FileLock, LockAttempt, LockMode},
    },
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::digest_hex;
mod receipts;

#[derive(Clone, Deserialize)]
struct Policy {
    path: PathBuf,
    #[serde(default)]
    entry_root: PathBuf,
    warm_size_bytes: u64,
    max_size_bytes: u64,
    maximum_age_hours: u64,
    #[serde(default)]
    mutation_locks: Vec<PathBuf>,
    #[serde(default)]
    protect_hardlinks: bool,
}

#[derive(Clone)]
pub(super) struct BlobCache {
    root: PathBuf,
    policy: Policy,
    namespace: String,
}

impl BlobCache {
    pub(super) fn installed() -> Result<Self> {
        let (root, policy) = Self::policy()?;
        Ok(Self {
            root: paths::capsem_home().join(root).join(&policy.path),
            policy,
            namespace: String::new(),
        })
    }

    fn policy() -> Result<(PathBuf, Policy)> {
        #[derive(Deserialize)]
        struct Contract {
            root: PathBuf,
            stages: BTreeMap<String, Policy>,
        }
        let mut contract: Contract = toml::from_str(include_str!("../../../../config/cache.toml"))?;
        let policy = contract
            .stages
            .remove("oci-images")
            .context("missing OCI cache contract")?;
        ensure!(policy.mutation_locks.len() == 1, "OCI cache requires one mutation lock");
        ensure!(
            policy.warm_size_bytes > 0 && policy.warm_size_bytes < policy.max_size_bytes,
            "invalid OCI cache capacity"
        );
        Ok((contract.root, policy))
    }

    #[cfg(test)]
    pub(super) fn at(root: &Path) -> Result<Self> {
        Ok(Self {
            root: root.to_owned(),
            policy: Self::policy()?.1,
            namespace: String::new(),
        })
    }

    pub(super) async fn prepare(&self) -> Result<()> {
        let cache = self.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&cache.root)?;
            ensure_private_dir(&cache.root)?;
            let root = ContainedDir::open_root(&cache.root)?;
            root.walk_creating(&cache.policy.entry_root, 0o700)?;
            root.descend_or_create("locks".as_ref(), 0o700)?;
            Ok(())
        })
        .await?
    }

    /// A registry manifest cannot grant access to another repository's cache.
    pub(super) fn for_repository(&self, reference: &oci_client::Reference) -> Self {
        let mut cache = self.clone();
        let repository = format!("{}/{}", reference.resolve_registry(), reference.repository());
        cache.namespace = format!("{:x}-", Sha256::digest(repository.as_bytes()));
        cache
    }

    pub(super) fn entry_name(&self, digest: &str) -> Result<String> {
        Ok(format!("{}{}", self.namespace, digest_hex(digest)?))
    }

    /// Fixed lock stripes bound lock-file growth while coalescing equal blobs.
    pub(super) async fn lease(&self, digest: &str) -> Result<FileLock> {
        let hex = digest_hex(digest)?;
        lock(self.root.join("locks").join(format!("{}.lock", &hex[..2]))).await
    }

    fn mutation_lock(&self) -> PathBuf {
        self.root.join(&self.policy.mutation_locks[0])
    }

    pub(super) async fn copy_hit(&self, digest: &str, size: u64, destination: &Path) -> Result<bool> {
        let lease = lock(self.mutation_lock()).await?;
        let cache = self.clone();
        let hex = self.entry_name(digest)?;
        let opened = tokio::task::spawn_blocking(move || -> Result<Option<File>> {
            let root = ContainedDir::open_root(&cache.root)?.walk(&cache.policy.entry_root)?;
            match root.open_file(hex.as_ref(), ContainedOpenOptions::read_only()) {
                Ok(file) => {
                    file.set_times(FileTimes::new().set_modified(SystemTime::now()))?;
                    Ok(Some(file))
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(error.into()),
            }
        })
        .await??;
        drop(lease);
        let Some(file) = opened else {
            return Ok(false);
        };
        let destination = destination.to_owned();
        let digest = digest.to_owned();
        // Holding the opened file is enough: pruning may unlink its name but
        // cannot invalidate this reader. Staging never shares writable inodes.
        tokio::task::spawn_blocking(move || verified_copy(file, &destination, &digest, size)).await?
    }

    /// Published roots use a separate immutable namespace. A layout's link
    /// protects the generation until its session share holds another link.
    pub(super) async fn link_root_hit(&self, digest: &str, size: u64, destination: &Path) -> Result<bool> {
        ensure!(
            self.policy.protect_hardlinks,
            "published roots require cache link protection"
        );
        let lease = lock(self.mutation_lock()).await?;
        let cache = self.clone();
        let name = format!("immutable-{}", self.entry_name(digest)?);
        let (digest, destination) = (digest.to_owned(), destination.to_owned());
        tokio::task::spawn_blocking(move || -> Result<bool> {
            let _lease = lease;
            let directory = ContainedDir::open_root(&cache.root)?.walk(&cache.policy.entry_root)?;
            let file = match directory.open_file(name.as_ref(), ContainedOpenOptions::read_only()) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(error) => return Err(error.into()),
            };
            let metadata = file.metadata()?;
            let valid = metadata.mode() & 0o777 == 0o444 && verified_bytes(file, &digest, size, |_| Ok(()))?;
            if !valid {
                ensure!(
                    metadata.nlink() == 1,
                    "active immutable cache payload is corrupt or writable"
                );
                directory.remove_non_directory(name.as_ref())?;
                return Ok(false);
            }
            let target = ContainedDir::open_root(destination.parent().context("root destination parent")?)?;
            directory.hard_link(
                name.as_ref(),
                &target,
                destination.file_name().context("root destination name")?,
            )?;
            directory
                .open_file(name.as_ref(), ContainedOpenOptions::read_only())?
                .set_times(FileTimes::new().set_modified(SystemTime::now()))?;
            Ok(true)
        })
        .await?
    }

    /// The producer has already verified this private, read-only payload.
    /// Publish its inode once; this namespace never overwrites a generation.
    pub(super) async fn publish_root(&self, digest: &str, source: &Path) -> Result<()> {
        ensure!(
            self.policy.protect_hardlinks,
            "published roots require cache link protection"
        );
        let lease = lock(self.mutation_lock()).await?;
        let cache = self.clone();
        let name = format!("immutable-{}", self.entry_name(digest)?);
        let source = source.to_owned();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let _lease = lease;
            let directory = ContainedDir::open_root(&cache.root)?.walk(&cache.policy.entry_root)?;
            let from = ContainedDir::open_root(source.parent().context("root source parent")?)?;
            let leaf = source.file_name().context("root source name")?;
            let file = from.open_file(leaf, ContainedOpenOptions::read_only())?;
            ensure!(
                file.metadata()?.mode() & 0o777 == 0o444,
                "published root must be read-only"
            );
            capsem_foundation::unix::fs::sync(file.as_fd())?;
            from.hard_link(leaf, &directory, name.as_ref())?;
            if let Err(error) = directory
                .sync()
                .map_err(anyhow::Error::from)
                .and_then(|()| cache.prune(&directory))
            {
                directory.remove_non_directory(name.as_ref())?;
                return Err(error);
            }
            Ok(())
        })
        .await?
    }

    pub(super) async fn publish(&self, digest: &str, source: &Path) -> Result<()> {
        let lease = lock(self.mutation_lock()).await?;
        let cache = self.clone();
        let hex = self.entry_name(digest)?;
        let source = source.to_owned();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let _lease = lease;
            let directory = ContainedDir::open_root(&cache.root)?.walk(&cache.policy.entry_root)?;
            let mut temporary = tempfile::Builder::new()
                .prefix(".partial-")
                .tempfile_in(directory.path())?;
            std::io::copy(&mut File::open(source)?, &mut temporary)?;
            temporary.as_file().sync_all()?;
            temporary.persist(directory.path().join(hex))?;
            cache.prune(&directory)
        })
        .await?
    }

    /// Metadata is bounded and rehashed before it can drive offline blob reads.
    pub(super) async fn read_metadata(&self, digest: &str, maximum: usize) -> Result<Option<Vec<u8>>> {
        let lease = lock(self.mutation_lock()).await?;
        let cache = self.clone();
        let name = self.entry_name(digest)?;
        let digest = digest.to_owned();
        tokio::task::spawn_blocking(move || -> Result<Option<Vec<u8>>> {
            let _lease = lease;
            let directory = ContainedDir::open_root(&cache.root)?.walk(&cache.policy.entry_root)?;
            let file = match directory.open_file(name.as_ref(), ContainedOpenOptions::read_only()) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error.into()),
            };
            let size = file.metadata()?.len();
            ensure!(size <= maximum as u64, "cached metadata exceeds limit");
            file.set_times(FileTimes::new().set_modified(SystemTime::now()))?;
            let mut bytes = Vec::with_capacity(usize::try_from(size)?);
            let valid = verified_bytes(file, &digest, size, |chunk| {
                bytes.extend_from_slice(chunk);
                Ok(())
            })?;
            Ok(valid.then_some(bytes))
        })
        .await?
    }

    pub(super) async fn publish_metadata(&self, digest: &str, bytes: &[u8]) -> Result<()> {
        ensure!(
            format!("sha256:{:x}", Sha256::digest(bytes)) == digest,
            "metadata digest mismatch"
        );
        let (root, bytes) = (self.root.clone(), bytes.to_owned());
        let source = tokio::task::spawn_blocking(move || -> Result<tempfile::NamedTempFile> {
            let mut source = tempfile::NamedTempFile::new_in(root)?;
            source.write_all(&bytes)?;
            Ok(source)
        })
        .await??;
        self.publish(digest, source.path()).await
    }

    fn prune(&self, directory: &ContainedDir) -> Result<()> {
        self.prune_for(directory, 0)
    }

    fn prune_for(&self, directory: &ContainedDir, reservation: u64) -> Result<()> {
        let mut entries = directory.entries()?;
        entries.sort_by_key(|entry| (entry.mtime_secs, entry.name.clone()));
        let mut used = entries.iter().try_fold(reservation, |used, entry| {
            used.checked_add(entry.size).context("OCI cache usage overflow")
        })?;
        let target = if used > self.policy.max_size_bytes {
            self.policy.warm_size_bytes
        } else {
            used
        };
        let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?.as_secs();
        for entry in entries {
            let stale = now.saturating_sub(entry.mtime_secs) > self.policy.maximum_age_hours * 3600;
            let name = entry.name.to_string_lossy();
            let partial = name.starts_with(".partial-");
            let receipt = name
                .strip_prefix("receipt-")
                .and_then(|key| super::CacheKey::parse(key).ok());
            let identity = name.strip_prefix("immutable-").unwrap_or(&name);
            let owned = partial
                || receipt.is_some()
                || (matches!(identity.len(), 64 | 129)
                    && identity
                        .split('-')
                        .all(|part| digest_hex(&format!("sha256:{part}")).is_ok()));
            if entry.kind == EntryKind::File && owned && (stale || used > target || partial) {
                if let Some(key) = &receipt {
                    if !matches!(receipts::read(directory, key), Ok(Some(_))) {
                        continue;
                    }
                }
                if self.policy.protect_hardlinks
                    && directory
                        .open_file(&entry.name, ContainedOpenOptions::read_only())?
                        .metadata()?
                        .nlink()
                        > 1
                {
                    continue;
                }
                directory.remove_non_directory(&entry.name)?;
                tracing::debug!(entry = %name, bytes = entry.size, stale, "pruned OCI cache entry");
                used = used.saturating_sub(entry.size);
            }
        }
        ensure!(
            used <= self.policy.max_size_bytes,
            "OCI cache capacity is held by protected or unrecognized entries"
        );
        Ok(())
    }
}

async fn lock(path: PathBuf) -> Result<FileLock> {
    poll_until(PollOpts::new("oci-cache-lock", Duration::from_secs(30)), || {
        let path = path.clone();
        async move {
            match tokio::task::spawn_blocking(move || try_acquire(&path, LockMode::Exclusive)).await {
                Ok(Ok(LockAttempt::Contended)) => None,
                Ok(Ok(LockAttempt::Acquired(lease))) => Some(Ok(lease)),
                Ok(Err(error)) => Some(Err(anyhow::Error::from(error))),
                Err(error) => Some(Err(anyhow::Error::from(error))),
            }
        }
    })
    .await
    .map_err(|error| anyhow::anyhow!("OCI cache lock timed out: {error:?}"))?
}

fn verified_copy(source: File, destination: &Path, expected: &str, size: u64) -> Result<bool> {
    if source.metadata()?.len() != size {
        return Ok(false);
    }
    let mut target = tempfile::NamedTempFile::new_in(destination.parent().context("blob destination parent")?)?;
    if !verified_bytes(source, expected, size, |bytes| target.write_all(bytes))? {
        return Ok(false);
    }
    target.persist_noclobber(destination)?;
    Ok(true)
}

fn verified_bytes(
    mut source: File,
    expected: &str,
    size: u64,
    mut consume: impl FnMut(&[u8]) -> std::io::Result<()>,
) -> Result<bool> {
    if source.metadata()?.len() != size {
        return Ok(false);
    }
    let mut digest = Sha256::new();
    let mut count = 0u64;
    let mut bytes = vec![0; 65536];
    loop {
        let read = source.read(&mut bytes)?;
        if read == 0 {
            break;
        }
        count += read as u64;
        if count > size {
            return Ok(false);
        }
        digest.update(&bytes[..read]);
        consume(&bytes[..read])?;
    }
    if count != size || format!("sha256:{:x}", digest.finalize()) != expected {
        return Ok(false);
    }
    Ok(true)
}

#[cfg(test)]
mod tests;
