use super::*;

#[tokio::test]
async fn capacity_retains_sparse_and_hardlinked_bytes_below_physical_limit() {
    use std::io::{Seek, SeekFrom};
    let root = super::super::tests::private_dir();
    let mut cache = BlobCache::at(root.path()).unwrap();
    cache.prepare().await.unwrap();
    cache.policy.warm_size_bytes = 512 * 1024;
    cache.policy.max_size_bytes = 1024 * 1024;
    let path = root.path().join("blobs").join("a".repeat(64));
    let mut file = File::create(&path).unwrap();
    file.seek(SeekFrom::Start(64 * 1024 * 1024)).unwrap();
    file.write_all(b"sparse bytes").unwrap();
    file.sync_all().unwrap();
    std::fs::hard_link(&path, root.path().join("locks/alias")).unwrap();
    let used = cache.usage().await.unwrap().allocated_bytes;
    assert!(used < cache.policy.max_size_bytes);
    let directory = ContainedDir::open_root(root.path())
        .unwrap()
        .walk(&cache.policy.entry_root)
        .unwrap();
    cache.prune(&directory).unwrap();
    assert!(
        path.exists(),
        "logical file length must not consume the physical budget"
    );
}

#[tokio::test]
async fn capacity_refuses_unprunable_control_bytes_outside_the_blob_directory() {
    let root = super::super::tests::private_dir();
    let mut cache = BlobCache::at(root.path()).unwrap();
    cache.prepare().await.unwrap();
    let baseline = cache.usage().await.unwrap().allocated_bytes;
    let control = root.path().join("locks/control");
    std::fs::write(&control, vec![7; 128 * 1024]).unwrap();
    cache.policy.warm_size_bytes = baseline + 1024;
    cache.policy.max_size_bytes = baseline + 64 * 1024;
    let directory = ContainedDir::open_root(root.path())
        .unwrap()
        .walk(&cache.policy.entry_root)
        .unwrap();
    assert!(
        cache.prune(&directory).is_err(),
        "control allocation is part of the same budget"
    );
    assert!(control.exists(), "control entries must not be broadly reclaimed");
}

#[tokio::test]
async fn a_receipt_shaped_name_does_not_make_unrecognized_bytes_prunable() {
    let root = super::super::tests::private_dir();
    let mut cache = BlobCache::at(root.path()).unwrap();
    cache.prepare().await.unwrap();
    let baseline = cache.usage().await.unwrap().allocated_bytes;
    let path = root
        .path()
        .join("blobs")
        .join(format!("receipt-oci-{}", "a".repeat(64)));
    let unknown = b"unrecognized receipt bytes";
    std::fs::write(&path, unknown).unwrap();
    cache.policy.warm_size_bytes = baseline.max(1);
    cache.policy.max_size_bytes = baseline + std::fs::metadata(&path).unwrap().blocks() * 256;
    let directory = ContainedDir::open_root(root.path())
        .unwrap()
        .walk(&cache.policy.entry_root)
        .unwrap();
    assert!(
        cache.prune(&directory).is_err(),
        "unknown bytes must cause capacity refusal, not broad deletion"
    );
    assert_eq!(std::fs::read(path).unwrap(), unknown);
}

fn identity(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

async fn set_one_payload_capacity(cache: &mut BlobCache, source: &Path) {
    let used = cache.usage().await.unwrap().allocated_bytes;
    cache.policy.warm_size_bytes = used;
    cache.policy.max_size_bytes = used + std::fs::metadata(source).unwrap().blocks() * 256;
}

#[tokio::test]
async fn receipt_publication_reserves_actual_allocation_and_keeps_its_live_temporary() {
    use super::super::receipts::{BlobKind, BlobRef, CacheReceipt};
    let root = super::super::tests::private_dir();
    let staging = tempfile::tempdir().unwrap();
    let manifest = b"manifest bytes";
    let config = b"configuration bytes";
    let manifest_digest = identity(manifest);
    let config_digest = identity(config);
    let reference = super::super::image_reference(&format!("localhost/team/image@{manifest_digest}")).unwrap();
    let mut cache = BlobCache::at(root.path()).unwrap().for_repository(&reference);
    cache.prepare().await.unwrap();
    for (digest, bytes) in [
        (&manifest_digest, manifest.as_slice()),
        (&config_digest, config.as_slice()),
    ] {
        cache.publish_metadata(digest, bytes).await.unwrap();
        std::fs::hard_link(
            root.path().join("blobs").join(cache.entry_name(digest).unwrap()),
            staging.path().join(digest_hex(digest).unwrap()),
        )
        .unwrap();
    }
    let receipt = CacheReceipt::new(
        super::super::CacheIdentity::new(&reference.to_string(), "amd64", super::super::RUNTIME_CONTRACT).unwrap(),
        &reference,
        super::super::Digest::parse(&manifest_digest).unwrap(),
        vec![
            BlobRef::new(&manifest_digest, manifest.len() as u64, BlobKind::Metadata).unwrap(),
            BlobRef::new(&config_digest, config.len() as u64, BlobKind::Private).unwrap(),
        ],
    )
    .unwrap();
    let bytes = receipt.encode().unwrap();
    let calibration = staging.path().join("receipt-allocation");
    std::fs::write(&calibration, &bytes).unwrap();
    let allocation = std::fs::metadata(&calibration).unwrap().blocks() * 512;
    assert!(
        allocation > bytes.len() as u64,
        "fixture must distinguish allocation from encoded length"
    );
    let used = cache.usage().await.unwrap().allocated_bytes;
    cache.policy.warm_size_bytes = used;
    cache.policy.max_size_bytes = used + bytes.len() as u64;
    assert!(
        cache.publish_receipt(receipt.clone()).await.is_err(),
        "receipt allocation must fit before success"
    );
    assert!(cache.read_receipt(&receipt.key()).await.unwrap().is_none());
    assert_eq!(
        std::fs::read_dir(root.path().join("blobs")).unwrap().count(),
        2,
        "failed live reservation cleans its temporary and keeps protected bytes"
    );
    cache.policy.max_size_bytes = used + allocation;
    cache.publish_receipt(receipt.clone()).await.unwrap();
    assert_eq!(
        cache
            .read_receipt(&receipt.key())
            .await
            .unwrap()
            .unwrap()
            .generation()
            .unwrap(),
        receipt.generation().unwrap()
    );
    assert!(cache.usage().await.unwrap().allocated_bytes <= cache.policy.max_size_bytes);
}

fn readonly_payload(path: &Path, bytes: &[u8]) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, bytes).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o444)).unwrap();
}

fn cached_root(root: &Path) -> PathBuf {
    let paths: Vec<_> = std::fs::read_dir(root.join("blobs"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(paths.len(), 1);
    paths[0].clone()
}

#[tokio::test]
async fn corrupt_inactive_root_is_a_miss_but_an_active_generation_cannot_be_replaced() {
    use std::os::unix::fs::PermissionsExt;
    let root = super::super::tests::private_dir();
    let staging = tempfile::tempdir().unwrap();
    let cache = BlobCache::at(root.path()).unwrap();
    cache.prepare().await.unwrap();
    let digest = identity(b"original");
    let producer = staging.path().join("producer");
    readonly_payload(&producer, b"original");
    cache.publish_root(&digest, &producer).await.unwrap();
    let stored = cached_root(root.path());
    // Simulate corruption by a trusted host writer outside the cache API.
    std::fs::set_permissions(&stored, std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::write(&stored, b"modified").unwrap();
    std::fs::set_permissions(&stored, std::fs::Permissions::from_mode(0o444)).unwrap();
    let destination = staging.path().join("refused");
    let error = cache.link_root_hit(&digest, 8, &destination).await.unwrap_err();
    assert!(format!("{error:#}").contains("active immutable cache payload"));
    assert!(!destination.exists());
    assert!(stored.exists(), "an active generation must not be silently replaced");
    std::fs::remove_file(producer).unwrap();
    assert!(!cache.link_root_hit(&digest, 8, &destination).await.unwrap());
    assert!(!stored.exists(), "an unheld corrupt generation is recoverable");
    let repaired = staging.path().join("repaired");
    readonly_payload(&repaired, b"original");
    cache.publish_root(&digest, &repaired).await.unwrap();
    assert!(cache.link_root_hit(&digest, 8, &destination).await.unwrap());
    assert_eq!(std::fs::read(destination).unwrap(), b"original");
}

#[tokio::test]
async fn root_publication_and_hits_share_one_readonly_inode_without_overwriting_holders() {
    let root = super::super::tests::private_dir();
    let staging = tempfile::tempdir().unwrap();
    let cache = BlobCache::at(root.path()).unwrap();
    cache.prepare().await.unwrap();
    let digest = identity(b"original");
    let producer = staging.path().join("producer");
    readonly_payload(&producer, b"original");
    cache.publish_root(&digest, &producer).await.unwrap();
    let holder = staging.path().join("holder");
    assert!(cache.link_root_hit(&digest, 8, &holder).await.unwrap());
    let inode = std::fs::metadata(&producer).unwrap().ino();
    assert_eq!(std::fs::metadata(&holder).unwrap().ino(), inode);
    assert_eq!(std::fs::metadata(cached_root(root.path())).unwrap().ino(), inode);
    assert_eq!(std::fs::metadata(&holder).unwrap().mode() & 0o777, 0o444);
    let replacement = staging.path().join("replacement");
    readonly_payload(&replacement, b"different");
    assert!(cache.publish_root(&digest, &replacement).await.is_err());
    assert_eq!(std::fs::read(&producer).unwrap(), b"original");
    assert_eq!(std::fs::read(&holder).unwrap(), b"original");
    std::fs::remove_file(producer).unwrap();
    assert_eq!(std::fs::read(holder).unwrap(), b"original");
}

#[tokio::test]
async fn root_retention_preserves_holders_and_reclaims_after_the_last_link() {
    let root = super::super::tests::private_dir();
    let staging = tempfile::tempdir().unwrap();
    let mut cache = BlobCache::at(root.path()).unwrap();
    cache.prepare().await.unwrap();
    let a = identity(b"aaaaaa");
    let producer = staging.path().join("a");
    readonly_payload(&producer, b"aaaaaa");
    cache.publish_root(&a, &producer).await.unwrap();
    set_one_payload_capacity(&mut cache, &producer).await;
    let retained = cached_root(root.path());
    File::open(&retained)
        .unwrap()
        .set_times(FileTimes::new().set_modified(SystemTime::UNIX_EPOCH))
        .unwrap();
    let b = staging.path().join("b");
    std::fs::write(&b, b"bbbbbb").unwrap();
    cache.publish(&identity(b"bbbbbb"), &b).await.unwrap();
    assert!(retained.exists(), "pruning must retain a root held by a layout/session");
    assert_eq!(std::fs::read(&producer).unwrap(), b"aaaaaa");
    std::fs::remove_file(&producer).unwrap();
    cache.publish(&identity(b"bbbbbb"), &b).await.unwrap();
    assert!(!retained.exists(), "unheld stale generations are eligible again");
    assert!(!cache.link_root_hit(&a, 6, &staging.path().join("late")).await.unwrap());
}

#[tokio::test]
async fn root_capacity_refusal_rolls_back_the_new_link_without_mutating_active_bytes() {
    let root = super::super::tests::private_dir();
    let staging = tempfile::tempdir().unwrap();
    let mut cache = BlobCache::at(root.path()).unwrap();
    cache.prepare().await.unwrap();
    let a = staging.path().join("a");
    readonly_payload(&a, b"aaaaaa");
    cache.publish_root(&identity(b"aaaaaa"), &a).await.unwrap();
    set_one_payload_capacity(&mut cache, &a).await;
    let b = staging.path().join("b");
    readonly_payload(&b, b"bbbbbb");
    assert!(cache.publish_root(&identity(b"bbbbbb"), &b).await.is_err());
    assert_eq!(std::fs::read(cached_root(root.path())).unwrap(), b"aaaaaa");
    assert_eq!(std::fs::read(a).unwrap(), b"aaaaaa");
    assert!(!cache
        .link_root_hit(&identity(b"bbbbbb"), 6, &staging.path().join("missing"))
        .await
        .unwrap());
}

#[tokio::test]
async fn root_owner_refuses_writable_payloads_and_never_follows_cache_links() {
    use std::os::unix::fs::symlink;
    let root = super::super::tests::private_dir();
    let staging = tempfile::tempdir().unwrap();
    let cache = BlobCache::at(root.path()).unwrap();
    cache.prepare().await.unwrap();
    let source = staging.path().join("source");
    std::fs::write(&source, b"original").unwrap();
    let digest = identity(b"original");
    assert!(cache.publish_root(&digest, &source).await.is_err());
    readonly_payload(&source, b"original");
    cache.publish_root(&digest, &source).await.unwrap();
    let stored = cached_root(root.path());
    std::fs::remove_file(&stored).unwrap();
    symlink(&source, &stored).unwrap();
    let destination = staging.path().join("refused");
    assert!(cache.link_root_hit(&digest, 8, &destination).await.is_err());
    assert!(!destination.exists());
    assert_eq!(std::fs::read(source).unwrap(), b"original");
}

#[tokio::test]
async fn existing_non_private_cache_is_rejected() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let error = BlobCache::at(root.path()).unwrap().prepare().await.unwrap_err();
    assert!(format!("{error:#}").contains("not 700"));
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn retention_leaves_active_staging_intact_and_removes_old_blobs() {
    let root = super::super::tests::private_dir();
    let staging = tempfile::tempdir().unwrap();
    let mut cache = BlobCache::at(root.path()).unwrap();
    cache.prepare().await.unwrap();
    let source = staging.path().join("source");
    let a = identity(b"aaaaaa");
    let b = identity(b"bbbbbb");
    std::fs::write(&source, b"aaaaaa").unwrap();
    cache.publish(&a, &source).await.unwrap();
    set_one_payload_capacity(&mut cache, &source).await;
    let active = staging.path().join("active");
    assert!(cache.copy_hit(&a, 6, &active).await.unwrap());
    File::open(root.path().join("blobs").join(digest_hex(&a).unwrap()))
        .unwrap()
        .set_times(FileTimes::new().set_modified(SystemTime::UNIX_EPOCH))
        .unwrap();
    std::fs::write(&source, b"bbbbbb").unwrap();
    cache.publish(&b, &source).await.unwrap();
    assert_eq!(std::fs::read(&active).unwrap(), b"aaaaaa");
    assert!(!cache.copy_hit(&a, 6, &staging.path().join("missing")).await.unwrap());
    assert!(cache.copy_hit(&b, 6, &staging.path().join("retained")).await.unwrap());
    assert_eq!(std::fs::read_dir(root.path().join("blobs")).unwrap().count(), 1);
}

#[tokio::test]
async fn cache_rejects_symlink_without_touching_target() {
    use std::os::unix::fs::symlink;
    let root = super::super::tests::private_dir();
    let staging = tempfile::tempdir().unwrap();
    let cache = BlobCache::at(root.path()).unwrap();
    cache.prepare().await.unwrap();
    let outside = staging.path().join("outside");
    std::fs::write(&outside, b"private").unwrap();
    let digest = identity(b"private");
    let path = root.path().join("blobs").join(digest_hex(&digest).unwrap());
    symlink(&outside, &path).unwrap();
    let destination = staging.path().join("destination");
    assert!(cache.copy_hit(&digest, 7, &destination).await.is_err());
    assert!(!destination.exists());
    assert_eq!(std::fs::read(&outside).unwrap(), b"private");
}

#[tokio::test]
async fn abandoned_partial_is_reclaimed_by_publication() {
    let root = super::super::tests::private_dir();
    let cache = BlobCache::at(root.path()).unwrap();
    cache.prepare().await.unwrap();
    let partial = root.path().join("blobs/.partial-abandoned");
    std::fs::write(&partial, b"incomplete").unwrap();
    let source = root.path().join("source");
    std::fs::write(&source, b"valid").unwrap();
    cache.publish(&identity(b"valid"), &source).await.unwrap();
    assert!(!partial.exists());
}
