use crate::oci::{
    receipts::{BlobKind, BlobRef, CacheReceipt},
    CacheIdentity, CacheInventory, CacheKey, CacheState, Digest, ImageCache,
};
use std::{
    fs::File,
    io::{Seek, SeekFrom, Write},
    os::unix::fs::MetadataExt,
    sync::Arc,
};

async fn stable_inventory(owner: &ImageCache) -> Arc<CacheInventory> {
    use capsem_foundation::poll::{poll_until, PollOpts};
    // Busy shared ancestors may conservatively invalidate an observation.
    // Each retry must obtain a real, still-valid inventory proof.
    poll_until(
        PollOpts::new("stable-cache-inventory", std::time::Duration::from_secs(2)),
        || async {
            match owner.refresh_inventory(64).await {
                Ok(()) => owner.inventory_snapshot().unwrap(),
                Err(error) if error.to_string() == "cache changed during inventory observation" => None,
                Err(error) => panic!("inventory observation failed: {error:#}"),
            }
        },
    )
    .await
    .unwrap()
}

async fn fixture() -> (tempfile::TempDir, ImageCache, CacheKey, CacheKey) {
    // A watched cache must not descend from every other test's busy temp
    // namespace. Keep a private sibling while retaining real path watches.
    let temporary = std::env::temp_dir();
    let parent = temporary
        .parent()
        .filter(|parent| *parent != std::path::Path::new("/"))
        .unwrap_or(std::path::Path::new("/var/tmp"));
    let root = crate::oci::tests::private_dir_in(parent);
    let owner = ImageCache::at(root.path()).unwrap();
    owner.inner.prepare().await.unwrap();
    let pin = format!("sha256:{}", "a".repeat(64));
    let first = format!("sha256:{}", "b".repeat(64));
    let alias = format!("sha256:{}", "c".repeat(64));
    let origin = crate::oci::image_reference(&format!("registry.example/team/image@{pin}")).unwrap();
    let scoped = owner.inner.for_repository(&origin);
    let path = root.path().join("blobs");
    std::fs::write(path.join(scoped.entry_name(&pin).unwrap()), b"metadata").unwrap();
    let sparse = path.join(scoped.entry_name(&first).unwrap());
    let mut file = File::create(&sparse).unwrap();
    file.seek(SeekFrom::Start(64 * 1024 * 1024)).unwrap();
    file.write_all(b"sparse").unwrap();
    file.sync_all().unwrap();
    let size = file.metadata().unwrap().len();
    std::fs::hard_link(&sparse, path.join(scoped.entry_name(&alias).unwrap())).unwrap();
    let mut keys = Vec::new();
    for contract in [1, 2] {
        let identity = CacheIdentity::new(&origin.to_string(), "amd64", contract).unwrap();
        let receipt = CacheReceipt::new(
            identity,
            &origin,
            Digest::parse(&pin).unwrap(),
            vec![
                BlobRef::new(&pin, 8, BlobKind::Metadata).unwrap(),
                BlobRef::new(&first, size, BlobKind::Private).unwrap(),
                BlobRef::new(&alias, size, BlobKind::Private).unwrap(),
            ],
        )
        .unwrap();
        let key = receipt.key();
        std::fs::write(
            path.join(format!("receipt-{}", key.as_str())),
            receipt.encode().unwrap(),
        )
        .unwrap();
        keys.push(key);
    }
    (root, owner, keys.remove(0), keys.remove(0))
}

#[tokio::test]
async fn retained_image_inventory_deduplicates_real_sparse_links_and_never_invents_readiness() {
    let (root, owner, first, second) = fixture().await;
    let facts = || {
        let mut facts = std::fs::read_dir(root.path().join("blobs"))
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                let metadata = entry.metadata().unwrap();
                (
                    entry.file_name(),
                    metadata.ino(),
                    metadata.len(),
                    metadata.nlink(),
                    metadata.mtime(),
                    metadata.mtime_nsec(),
                )
            })
            .collect::<Vec<_>>();
        facts.sort();
        facts
    };
    let before = facts();
    let observed = owner.inventory().await.unwrap();
    assert_eq!(facts(), before, "metadata accounting must not mutate retained files");
    assert_eq!(observed.usage, owner.usage().await.unwrap());
    assert_eq!(observed.images.len(), 2);
    assert!(observed.reference_graph_complete);
    assert_eq!(observed.invalid_receipts, 0);
    for key in [first, second] {
        let row = observed.images.iter().find(|row| row.receipt.key() == key).unwrap();
        let receipt = std::fs::metadata(root.path().join("blobs").join(format!("receipt-{}", key.as_str()))).unwrap();
        assert_eq!(row.snapshot.state, CacheState::Unknown);
        assert!(row.snapshot.verified_at_unix_ns.is_none());
        assert_eq!(row.missing_blobs, 0);
        assert_eq!(row.reclaimable_allocated_bytes, receipt.blocks() * 512);
        assert_eq!(
            row.allocated_bytes,
            row.shared_allocated_bytes + row.reclaimable_allocated_bytes
        );
        assert!(row.allocated_bytes < 64 * 1024 * 1024);
        assert!(row.logical_bytes > 64 * 1024 * 1024 && row.logical_bytes < 65 * 1024 * 1024);
    }
}

#[tokio::test]
async fn inventory_reports_missing_dependencies_and_busy_materialization_without_reclaim() {
    let (root, owner, first, _) = fixture().await;
    let held = owner.inner.materialization_lease(&first).await.unwrap();
    let busy = owner.inventory().await.unwrap();
    assert!(busy
        .images
        .iter()
        .all(|row| row.busy && row.reclaimable_allocated_bytes == 0));
    drop(held);
    let receipt = &busy.images[0].receipt;
    let scoped = owner
        .inner
        .for_repository(&crate::oci::image_reference(receipt.origin()).unwrap());
    for blob in receipt.blobs().iter().filter(|blob| blob.kind == BlobKind::Private) {
        std::fs::remove_file(root.path().join("blobs").join(scoped.entry_name(&blob.digest).unwrap())).unwrap();
    }
    let missing = owner.inventory().await.unwrap();
    assert!(missing
        .images
        .iter()
        .all(|row| row.missing_blobs == 2 && row.snapshot.state == CacheState::Unknown));
    assert_eq!(missing.usage, owner.usage().await.unwrap());
}

#[tokio::test]
async fn reused_materialization_controls_invalidate_inventory_on_acquire_and_after_release() {
    let (_root, owner, first, _) = fixture().await;
    // Reuse existing controls: flock transitions themselves emit no file
    // notification, so creating a fresh lock name would hide the mistake.
    drop(owner.inner.materialization_lease(&first).await.unwrap());
    let quiet = stable_inventory(&owner).await;
    assert!(quiet
        .images
        .iter()
        .all(|image| !image.busy && image.reclaimable_allocated_bytes > 0));
    let held = owner.inner.materialization_lease(&first).await.unwrap();
    assert!(
        owner.inventory_snapshot().unwrap().is_none(),
        "an owned live lease must obsolete the reclaim estimate"
    );
    let busy = stable_inventory(&owner).await;
    assert!(busy
        .images
        .iter()
        .all(|image| image.busy && image.reclaimable_allocated_bytes == 0));
    drop(held);
    assert!(
        owner.inventory_snapshot().unwrap().is_none(),
        "lease release must obsolete the busy observation"
    );
    let released = stable_inventory(&owner).await;
    assert!(released
        .images
        .iter()
        .all(|image| !image.busy && image.reclaimable_allocated_bytes > 0));
}

#[tokio::test]
async fn cancelled_partial_materialization_releases_its_barrier_before_invalidating_inventory() {
    use capsem_foundation::{
        poll::{poll_until, PollOpts},
        unix::lock::{try_acquire_existing, LockAttempt, LockMode},
    };
    let (root, owner, first, _) = fixture().await;
    drop(owner.inner.materialization_lease(&first).await.unwrap());
    let control = root
        .path()
        .join("locks")
        .join(format!("materialize-{}.lock", first.as_str()));
    let LockAttempt::Acquired(other) = try_acquire_existing(&control, LockMode::Exclusive).unwrap() else {
        panic!("fixture image control unexpectedly held");
    };
    stable_inventory(&owner).await;
    let before = owner.snapshot(&first).unwrap().epoch;
    let source = owner.clone();
    let target = first.clone();
    let pending = tokio::spawn(async move { source.inner.materialization_lease(&target).await });
    poll_until(
        PollOpts::new("partial-materialization", std::time::Duration::from_secs(1)),
        || async {
            let barrier = root.path().join("locks/materialization.lock");
            match try_acquire_existing(&barrier, LockMode::Exclusive).unwrap() {
                LockAttempt::Contended => (owner.snapshot(&first).unwrap().epoch > before).then_some(()),
                LockAttempt::Acquired(lease) => {
                    drop(lease);
                    None
                }
            }
        },
    )
    .await
    .unwrap();
    assert!(owner.inventory_snapshot().unwrap().is_none());
    assert!(stable_inventory(&owner).await.images.iter().all(|row| row.busy));
    pending.abort();
    assert!(matches!(pending.await, Err(error) if error.is_cancelled()));
    assert!(owner.inventory_snapshot().unwrap().is_none());
    let LockAttempt::Acquired(barrier) =
        try_acquire_existing(&root.path().join("locks/materialization.lock"), LockMode::Exclusive).unwrap()
    else {
        panic!("cancelled partial acquisition retained the barrier");
    };
    drop(barrier);
    drop(other);
    assert!(stable_inventory(&owner).await.images.iter().all(|row| !row.busy));
}

#[tokio::test]
async fn an_owned_unassociated_alias_is_classified_independently_of_enumeration_order() {
    let root = crate::oci::tests::private_dir();
    let owner = ImageCache::at(root.path()).unwrap();
    owner.inner.prepare().await.unwrap();
    let unknown = root.path().join("blobs/unknown-first");
    std::fs::write(&unknown, b"legacy bytes").unwrap();
    std::fs::hard_link(&unknown, root.path().join("blobs").join("a".repeat(64))).unwrap();
    let inventory = owner.inventory().await.unwrap();
    assert_eq!(
        inventory.unassociated_allocated_bytes,
        std::fs::metadata(unknown).unwrap().blocks() * 512
    );
    assert_eq!(inventory.unmanaged_allocated_bytes, 0);
}

#[tokio::test]
async fn malformed_and_linked_receipts_are_counted_without_granting_reclaim_or_following_targets() {
    let (root, owner, _, _) = fixture().await;
    let unknown = root.path().join("blobs/unknown");
    std::fs::write(&unknown, b"unmanaged bytes").unwrap();
    let partial = root.path().join("blobs/.partial-download");
    std::fs::write(&partial, b"partial bytes").unwrap();
    let forged = root.path().join(format!("blobs/receipt-oci-{}", "d".repeat(64)));
    std::fs::write(&forged, b"malformed").unwrap();
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("secret");
    std::fs::write(&target, vec![7; 1024 * 1024]).unwrap();
    std::os::unix::fs::symlink(
        &target,
        root.path().join(format!("blobs/receipt-oci-{}", "e".repeat(64))),
    )
    .unwrap();
    let observed = owner.inventory().await.unwrap();
    assert!(!observed.reference_graph_complete);
    assert_eq!(observed.invalid_receipts, 2);
    assert_eq!(observed.images.len(), 2);
    assert!(observed.images.iter().all(|row| row.reclaimable_allocated_bytes == 0));
    assert!(observed.unassociated_allocated_bytes >= std::fs::metadata(&partial).unwrap().blocks() * 512);
    assert!(observed.unmanaged_allocated_bytes >= std::fs::metadata(&unknown).unwrap().blocks() * 512);
    assert!(observed.control_allocated_bytes > 0);
    assert_eq!(
        observed.usage.allocated_bytes,
        observed.associated_allocated_bytes
            + observed.control_allocated_bytes
            + observed.unassociated_allocated_bytes
            + observed.unmanaged_allocated_bytes
    );
    assert_eq!(std::fs::read(&target).unwrap(), vec![7; 1024 * 1024]);
    assert_eq!(std::fs::read(forged).unwrap(), b"malformed");
}
