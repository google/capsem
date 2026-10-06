//! Exact cleanup is admitted once and every step is durably retryable.

use super::super::super::RemovalResult;
use super::{
    journal::{self, Journal, Step},
    *,
};

impl BlobCache {
    pub(in super::super::super) async fn apply_removal(
        &self,
        key: &CacheKey,
        token: &str,
        reason: &str,
    ) -> Result<RemovalResult> {
        let lease = lock_with(self.mutation_lock(), LockAccess::Existing, LockMode::Exclusive).await?;
        let (cache, key, token, reason) = (self.clone(), key.clone(), token.to_owned(), reason.to_owned());
        tokio::task::spawn_blocking(move || {
            let _lease = lease;
            let root = ContainedDir::open_root(&cache.root)?;
            root.validate_private()?;
            let directory = root.walk(&cache.policy.entry_root)?;
            apply(&cache, &root, &directory, &key, &token, &reason)
        })
        .await?
    }
}

fn apply(
    cache: &BlobCache,
    root: &ContainedDir,
    directory: &ContainedDir,
    key: &CacheKey,
    token: &str,
    reason: &str,
) -> Result<RemovalResult> {
    let mut journal = if let Some(journal) = journal::load(root, token)? {
        journal.validate(cache, key, token, reason)?;
        if journal.result.complete {
            return Ok(journal.result);
        }
        journal
    } else {
        let plan = preview(cache, root, directory, key)?;
        ensure!(
            plan.allowed(),
            "cache is protected by retained roots or materialization"
        );
        ensure!(plan.token() == token, "removal preview is stale");
        Journal::new(
            plan,
            &receipts::read(directory, key)?.context("receipt disappeared before intent")?,
            reason,
        )?
    };
    let locks = root.descend("locks".as_ref())?;
    let _barrier = match try_acquire(&locks.path().join("materialization.lock"), LockMode::Exclusive)? {
        LockAttempt::Acquired(lease) => lease,
        LockAttempt::Contended => anyhow::bail!("cache has an active materialization"),
    };
    ensure!(!materializing_with(root, false)?, "cache has an active materialization");
    // Check after taking the barrier: no newly started materialization can
    // alter reference ownership between revalidation and intent publication.
    if journal::load(root, token)?.is_none() {
        let current = preview_without_barrier(cache, root, directory, key)?;
        ensure!(
            current.token() == token && current.allowed(),
            "removal preview is stale or protected"
        );
    } else if let Some(current) = receipts::read(directory, key)? {
        ensure!(
            !journal.done.contains(&format!("receipt-{}", key.as_str()))
                && current.generation()? == journal.witness.generation,
            "image was republished after interrupted removal"
        );
    }
    let retained = other_names(cache, directory, key)?;
    preflight(directory, &journal, &retained)?;
    cache
        .tracking
        .lock()
        .map_err(|_| anyhow::anyhow!("OCI tracking lock poisoned"))?
        .invalidate()?;
    journal.save(root)?;
    let receipt_name = format!("receipt-{}", key.as_str());
    let receipt_file = journal.witness.receipt.clone();
    remove_step(
        root,
        directory,
        &mut journal,
        Step {
            name: receipt_name,
            file: receipt_file,
        },
    )?;
    #[cfg(test)]
    if std::env::var("CAPSEM_OCI_REMOVE_TEST_STOP").as_deref() == Ok("after_receipt") {
        std::process::exit(91);
    }
    let blobs = journal.witness.blobs.clone();
    for blob in blobs {
        if journal.done.contains(&blob.name) {
            continue;
        }
        if blob.shared || retained.contains(&blob.name) || blob.file.as_ref().is_some_and(|file| file.links > 1) {
            if journal.pending.as_ref().is_some_and(|step| step.name == blob.name) {
                journal.pending = None;
            }
            journal.result.retained_entries += 1;
            journal.done.insert(blob.name);
            journal.save(root)?;
        } else if let Some(file) = blob.file {
            remove_step(root, directory, &mut journal, Step { name: blob.name, file })?;
        } else {
            journal.result.already_missing_entries += 1;
            journal.done.insert(blob.name);
            journal.save(root)?;
        }
    }
    journal.result.complete = true;
    journal.save(root)?;
    Ok(journal.result)
}

fn preflight(directory: &ContainedDir, journal: &Journal, retained: &BTreeSet<String>) -> Result<()> {
    for blob in &journal.witness.blobs {
        let current = current_file(directory, &blob.name)?;
        ensure!(
            blob.kind != BlobKind::ImmutableRoot || current.as_ref().is_none_or(|file| file.links == 1),
            "cache root became protected during interrupted removal"
        );
        if journal.done.contains(&blob.name)
            || blob.shared
            || retained.contains(&blob.name)
            || blob.file.as_ref().is_some_and(|file| file.links > 1)
        {
            continue;
        }
        ensure!(
            current.is_none() || current == blob.file,
            "cache inode changed during interrupted removal"
        );
    }
    Ok(())
}

fn current_file(directory: &ContainedDir, name: &str) -> Result<Option<FileState>> {
    match directory.open_file(name.as_ref(), ContainedOpenOptions::read_only()) {
        Ok(file) => Ok(Some(FileState::from_metadata(&file.metadata()?)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn remove_step(root: &ContainedDir, directory: &ContainedDir, journal: &mut Journal, step: Step) -> Result<()> {
    if journal.done.contains(&step.name) {
        return Ok(());
    }
    let current = current_file(directory, &step.name)?;
    if let Some(current) = current {
        ensure!(current == step.file, "cache inode changed during removal");
        journal.pending = Some(step.clone());
        journal.save(root)?;
        directory.remove_non_directory(step.name.as_ref())?;
        directory.sync()?;
        #[cfg(test)]
        if std::env::var("CAPSEM_OCI_REMOVE_TEST_STOP").as_deref() == Ok("after_unlink") {
            std::process::exit(92);
        }
        journal.result.removed_entries += 1;
        if step.file.links == 1 {
            journal.result.removed_allocated_bytes = journal
                .result
                .removed_allocated_bytes
                .checked_add(step.file.allocated)
                .context("removal allocation overflow")?;
        }
    } else {
        journal.result.already_missing_entries += 1;
    }
    journal.pending = None;
    journal.done.insert(step.name);
    journal.save(root)?;
    Ok(())
}
