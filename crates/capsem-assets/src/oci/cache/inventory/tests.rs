use crate::oci::{
    receipts::{BlobKind, BlobRef, CacheReceipt},
    CacheIdentity, CacheKey, CacheState, Digest, ImageCache,
};
use std::{
    fs::File,
    io::{Seek, SeekFrom, Write},
    os::unix::fs::MetadataExt,
};

async fn fixture() -> (tempfile::TempDir, ImageCache, CacheKey, CacheKey) {
    let root = crate::oci::tests::private_dir();
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
