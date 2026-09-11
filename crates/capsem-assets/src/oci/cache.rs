//! Verified OCI blob retention. Policy is shared with the repository cache owner.

use std::{
    collections::BTreeMap,
    fs::{File, FileTimes},
    io::{Read, Write},
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

    fn prune(&self, directory: &ContainedDir) -> Result<()> {
        let mut entries = directory.entries()?;
        entries.sort_by_key(|entry| (entry.mtime_secs, entry.name.clone()));
        let mut used: u64 = entries.iter().map(|entry| entry.size).sum();
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
            let owned = partial
                || (matches!(name.len(), 64 | 129)
                    && name
                        .split('-')
                        .all(|part| digest_hex(&format!("sha256:{part}")).is_ok()));
            if entry.kind == EntryKind::File && owned && (stale || used > target || partial) {
                std::fs::remove_file(directory.path().join(&entry.name))?;
                tracing::debug!(entry = %name, bytes = entry.size, stale, "pruned OCI cache entry");
                used = used.saturating_sub(entry.size);
            }
        }
        ensure!(
            used <= self.policy.max_size_bytes,
            "OCI cache capacity is held by unrecognized entries"
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

fn verified_copy(mut source: File, destination: &Path, expected: &str, size: u64) -> Result<bool> {
    if source.metadata()?.len() != size {
        return Ok(false);
    }
    let mut target = tempfile::NamedTempFile::new_in(destination.parent().context("blob destination parent")?)?;
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
        target.write_all(&bytes[..read])?;
    }
    if count != size || format!("sha256:{:x}", digest.finalize()) != expected {
        return Ok(false);
    }
    target.persist_noclobber(destination)?;
    Ok(true)
}

#[cfg(test)]
mod tests;
