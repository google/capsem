use super::super::tests::{digest, Registry};
use super::super::*;

#[tokio::test]
async fn ancestor_rebinding_invalidates_ready_and_missing_cache_observations() {
    use std::os::unix::fs::DirBuilderExt;
    let registry = Registry::start(|_, _| {}).await;
    let parent = tempfile::tempdir().unwrap();
    let root = super::super::super::tests::private_dir();
    let ancestor = root.path().join("ancestor");
    let cache_root = ancestor.join("cache");
    let mut puller = registry.puller();
    puller.cache = Some(BlobCache::at(&cache_root).unwrap());
    let image = puller.pull(&registry.reference(), parent.path()).await.unwrap();
    let key = image.cache_identity().key();
    drop(image);
    registry.task.abort();
    assert_eq!(
        puller.reconcile_cache(&key, parent.path()).await.unwrap().state,
        super::super::super::CacheState::Ready
    );
    std::fs::rename(&ancestor, root.path().join("previous-ancestor")).unwrap();
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&cache_root)
        .unwrap();
    assert!(
        puller.cache_snapshot(&key).unwrap().verification_pending,
        "an unchanged old root inode cannot preserve readiness at a replaced path"
    );
    assert_eq!(
        puller.reconcile_cache(&key, parent.path()).await.unwrap().state,
        super::super::super::CacheState::Missing
    );
    std::fs::rename(&ancestor, root.path().join("empty-ancestor")).unwrap();
    std::fs::rename(root.path().join("previous-ancestor"), &ancestor).unwrap();
    assert!(
        puller.cache_snapshot(&key).unwrap().verification_pending,
        "restoring the original ancestor must invalidate the missing observation"
    );
    assert_eq!(
        puller.reconcile_cache(&key, parent.path()).await.unwrap().state,
        super::super::super::CacheState::Ready
    );
}

#[tokio::test]
async fn reconciling_another_image_preserves_readiness_and_retained_inode_facts() {
    use std::os::unix::fs::MetadataExt;
    let first = Registry::start(|_, _| {}).await;
    let second = Registry::start(|_, _| {}).await;
    let parent = tempfile::tempdir().unwrap();
    let root = super::super::super::tests::private_dir();
    let mut puller = first.puller();
    puller.cache = Some(BlobCache::at(root.path()).unwrap());
    let image = puller.pull(&first.reference(), parent.path()).await.unwrap();
    let first_key = image.cache_identity().key();
    let filesystem = puller
        .fetch_rootfs(
            &first.published_filesystem(&image.source_digest, None),
            &ContentDigest::parse(&image.source_digest).unwrap(),
            parent.path(),
        )
        .await
        .unwrap();
    puller.retain_cached_root(&first_key, &filesystem).await.unwrap();
    drop(filesystem);
    drop(image);
    let mut other = second.puller();
    other.cache = puller.cache.clone();
    let image = other.pull(&second.reference(), parent.path()).await.unwrap();
    let second_key = image.cache_identity().key();
    drop(image);
    first.task.abort();
    second.task.abort();
    let facts = || {
        let mut facts = std::fs::read_dir(root.path().join("blobs"))
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                let metadata = entry.metadata().unwrap();
                (
                    entry.file_name(),
                    metadata.ino(),
                    metadata.mtime(),
                    metadata.mtime_nsec(),
                    metadata.mode(),
                    metadata.nlink(),
                )
            })
            .collect::<Vec<_>>();
        facts.sort();
        facts
    };
    let before = facts();
    let ready = puller.reconcile_cache(&first_key, parent.path()).await.unwrap();
    assert_eq!(ready.state, super::super::super::CacheState::Ready);
    assert_eq!(
        facts(),
        before,
        "observation must not refresh, link or replace retained payloads"
    );
    assert_eq!(
        other.reconcile_cache(&second_key, parent.path()).await.unwrap().state,
        super::super::super::CacheState::Ready
    );
    assert_eq!(
        puller.cache_snapshot(&first_key).unwrap(),
        ready,
        "another key's proof must preserve this observation and epoch"
    );
    assert_eq!(facts(), before);
    assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 0);
    let immutable = std::fs::read_dir(root.path().join("blobs"))
        .unwrap()
        .map(Result::unwrap)
        .find(|entry| entry.file_name().to_string_lossy().starts_with("immutable-"))
        .unwrap()
        .path();
    std::fs::set_permissions(&immutable, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(&immutable, b"corrupt immutable filesystem").unwrap();
    std::fs::set_permissions(&immutable, std::fs::Permissions::from_mode(0o444)).unwrap();
    let corrupt = facts();
    assert!(puller.reconcile_cache(&first_key, parent.path()).await.is_err());
    assert_eq!(
        facts(),
        corrupt,
        "observation must preserve corrupt evidence without unlinking it"
    );
}

#[tokio::test]
async fn readiness_starts_pending_and_external_changes_invalidate_verified_bytes() {
    let registry = Registry::start(|_, _| {}).await;
    let parent = tempfile::tempdir().unwrap();
    let root = super::super::super::tests::private_dir();
    let mut puller = registry.puller();
    puller.cache = Some(BlobCache::at(root.path()).unwrap());
    let image = puller.pull(&registry.reference(), parent.path()).await.unwrap();
    let key = image.cache_identity().key();
    let reference = registry.published_filesystem(&image.source_digest, None);
    let filesystem = puller
        .fetch_rootfs(
            &reference,
            &ContentDigest::parse(&image.source_digest).unwrap(),
            parent.path(),
        )
        .await
        .unwrap();
    puller.retain_cached_root(&key, &filesystem).await.unwrap();
    drop(filesystem);
    drop(image);
    assert!(puller.cache_snapshot(&key).unwrap().verification_pending);
    registry.task.abort();
    let checked = puller.reconcile_cache(&key, parent.path()).await.unwrap();
    assert_eq!(checked.state, super::super::super::CacheState::Ready);
    assert!(checked.verified_at_unix_ns.is_some());
    assert_eq!(puller.cache_snapshot(&key).unwrap(), checked);
    let mut fresh = registry.puller();
    fresh.cache = Some(BlobCache::at(root.path()).unwrap());
    assert!(fresh.cached_receipt(&key).await.unwrap().is_some());
    let startup = fresh.cache_snapshot(&key).unwrap();
    assert!(
        startup.verification_pending,
        "a fresh owner cannot recover readiness from receipt metadata"
    );
    assert!(startup.verified_at_unix_ns.is_none());
    assert_eq!(
        puller.cache_snapshot(&key).unwrap(),
        checked,
        "metadata reads must remain quiet"
    );
    let cache = puller
        .cache
        .as_ref()
        .unwrap()
        .for_repository(&image_reference(&registry.reference()).unwrap());
    let layer = root
        .path()
        .join("blobs")
        .join(cache.entry_name(&digest(&registry.layer)).unwrap());
    std::fs::write(&layer, vec![b'x'; registry.layer.len()]).unwrap();
    let invalid = puller.cache_snapshot(&key).unwrap();
    assert!(invalid.verification_pending);
    assert_eq!(invalid.state, super::super::super::CacheState::Unknown);
    assert!(invalid.epoch > checked.epoch);
    assert!(invalid.verified_at_unix_ns.is_none());
    assert!(puller.reconcile_cache(&key, parent.path()).await.is_err());
    assert_eq!(
        puller.cache_snapshot(&key).unwrap().state,
        super::super::super::CacheState::Partial
    );
    assert_eq!(
        puller.cache_snapshot(&key).unwrap().reason,
        Some(crate::oci::CacheReason::IntegrityInvalid)
    );
    std::fs::write(&layer, &registry.layer).unwrap();
    assert!(
        puller.cache_snapshot(&key).unwrap().verification_pending,
        "external repair must invalidate a partial observation before reconciliation"
    );
    let repaired = puller.reconcile_cache(&key, parent.path()).await.unwrap();
    assert_eq!(repaired.state, super::super::super::CacheState::Ready);
    let receipt = root.path().join("blobs").join(format!("receipt-{}", key.as_str()));
    let bytes = std::fs::read(&receipt).unwrap();
    std::fs::write(&receipt, bytes).unwrap();
    assert!(
        puller.cache_snapshot(&key).unwrap().verification_pending,
        "same-content receipt rewrites invalidate the observation"
    );
    puller.reconcile_cache(&key, parent.path()).await.unwrap();
    std::fs::remove_file(&layer).unwrap();
    std::os::unix::fs::symlink("foreign", &layer).unwrap();
    assert!(puller.cache_snapshot(&key).unwrap().verification_pending);
    assert!(puller.reconcile_cache(&key, parent.path()).await.is_err());
    assert_eq!(
        puller.cache_snapshot(&key).unwrap().state,
        super::super::super::CacheState::Partial
    );
    std::fs::remove_file(&layer).unwrap();
    std::fs::write(&layer, &registry.layer).unwrap();
    assert!(
        puller.cache_snapshot(&key).unwrap().verification_pending,
        "replacement of a nonregular required entry must invalidate partial state"
    );
}

#[tokio::test]
async fn missing_receipt_watch_invalidates_external_creation_and_directory_replacement() {
    let registry = Registry::start(|_, _| {}).await;
    let parent = tempfile::tempdir().unwrap();
    let root = super::super::super::tests::private_dir();
    let mut puller = registry.puller();
    puller.cache = Some(BlobCache::at(root.path()).unwrap());
    let image = puller.pull(&registry.reference(), parent.path()).await.unwrap();
    let key = image.cache_identity().key();
    drop(image);
    registry.task.abort();
    let receipt = root.path().join("blobs").join(format!("receipt-{}", key.as_str()));
    let bytes = std::fs::read(&receipt).unwrap();
    std::fs::remove_file(&receipt).unwrap();
    let missing = puller.reconcile_cache(&key, parent.path()).await.unwrap();
    assert_eq!(missing.state, super::super::super::CacheState::Missing);
    assert_eq!(missing.reason, Some(crate::oci::CacheReason::ReceiptMissing));
    assert_eq!(puller.cache_snapshot(&key).unwrap(), missing);
    std::fs::write(&receipt, &bytes).unwrap();
    let invalid = puller.cache_snapshot(&key).unwrap();
    assert!(invalid.verification_pending);
    assert!(invalid.epoch > missing.epoch);
    std::fs::remove_file(&receipt).unwrap();
    puller.reconcile_cache(&key, parent.path()).await.unwrap();
    let directory = root.path().join("blobs");
    std::fs::rename(&directory, root.path().join("previous-blobs")).unwrap();
    std::fs::create_dir(&directory).unwrap();
    assert!(puller.cache_snapshot(&key).unwrap().verification_pending);
}

#[tokio::test]
async fn cancelled_reconciliation_invalidates_its_epoch() {
    let registry = Registry::start(|_, _| {}).await;
    let parent = tempfile::tempdir().unwrap();
    let root = super::super::super::tests::private_dir();
    let mut puller = registry.puller();
    puller.cache = Some(BlobCache::at(root.path()).unwrap());
    let image = puller.pull(&registry.reference(), parent.path()).await.unwrap();
    let key = image.cache_identity().key();
    drop(image);
    registry.task.abort();
    puller.reconcile_cache(&key, parent.path()).await.unwrap();
    let lease = puller.cache.as_ref().unwrap().mutation_lease().await.unwrap();
    let before = puller.cache_snapshot(&key).unwrap().epoch;
    let mut operation = Box::pin(puller.reconcile_cache(&key, parent.path()));
    assert!(futures::poll!(operation.as_mut()).is_pending());
    let active = puller.cache_snapshot(&key).unwrap().epoch;
    assert!(active > before, "verification must issue a new epoch before waiting");
    drop(operation);
    let cancelled = puller.cache_snapshot(&key).unwrap();
    assert!(
        cancelled.epoch > active,
        "cancellation must obsolete a pending verification ticket"
    );
    assert!(cancelled.verification_pending);
    drop(lease);
    assert_eq!(
        puller.reconcile_cache(&key, parent.path()).await.unwrap().state,
        super::super::super::CacheState::Ready
    );
}

#[tokio::test]
async fn own_mutation_invalidates_before_publication_finishes() {
    let registry = Registry::start(|_, _| {}).await;
    let parent = tempfile::tempdir().unwrap();
    let root = super::super::super::tests::private_dir();
    let mut puller = registry.puller();
    puller.cache = Some(BlobCache::at(root.path()).unwrap());
    let image = puller.pull(&registry.reference(), parent.path()).await.unwrap();
    let key = image.cache_identity().key();
    drop(image);
    registry.task.abort();
    let ready = puller.reconcile_cache(&key, parent.path()).await.unwrap();
    let cache = puller.cache.as_ref().unwrap();
    let lease = cache.mutation_lease().await.unwrap();
    let invalid = puller.cache_snapshot(&key).unwrap();
    assert!(invalid.epoch > ready.epoch);
    assert!(
        invalid.verification_pending,
        "must invalidate while lease held, before mutation or success"
    );
    drop(lease);
}
