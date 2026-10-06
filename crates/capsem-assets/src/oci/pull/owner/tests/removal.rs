use super::*;
use std::os::unix::fs::MetadataExt;

#[tokio::test]
async fn removal_preview_keeps_bytes_referenced_by_another_runtime_receipt() {
    let registry = Registry::start(|_, _| {}).await;
    let parent = tempfile::tempdir().unwrap();
    let root = crate::oci::tests::private_dir();
    let owner = ImageCache::at(root.path()).unwrap();
    let puller = registry.puller().with_cache(&owner);
    let image = puller.pull(&registry.reference(), parent.path()).await.unwrap();
    let key = image.cache_identity().key();
    drop(image);
    registry.task.abort();
    let before = owner.preview_removal(&key).await.unwrap();
    let receipt = puller.cached_receipt(&key).await.unwrap().unwrap();
    let other_key = crate::oci::CacheIdentity::new(
        &receipt.identity().image().to_string(),
        receipt.identity().architecture(),
        RUNTIME_CONTRACT + 1,
    )
    .unwrap()
    .key();
    let mut record: serde_json::Value = serde_json::from_slice(&receipt.encode().unwrap()).unwrap();
    record["key"] = serde_json::json!(other_key.as_str());
    record["runtime_contract"] = serde_json::json!(RUNTIME_CONTRACT + 1);
    let other = crate::oci::receipts::CacheReceipt::decode(&serde_json::to_vec(&record).unwrap(), &other_key).unwrap();
    owner.inner.publish_receipt(other).await.unwrap();
    let shared = owner.preview_removal(&key).await.unwrap();
    assert!(shared.allowed());
    assert_ne!(
        before.token(),
        shared.token(),
        "new reference ownership must obsolete the old preview"
    );
    let allocated = std::fs::metadata(root.path().join("blobs").join(format!("receipt-{}", key.as_str())))
        .unwrap()
        .blocks()
        * 512;
    assert_eq!(
        shared.reclaimable_bytes(),
        allocated,
        "all data blobs are shared; only this receipt is reclaimable"
    );
    let names = std::fs::read_dir(root.path().join("blobs"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    owner.preview_removal(&key).await.unwrap();
    assert_eq!(
        std::fs::read_dir(root.path().join("blobs"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>(),
        names
    );
}

#[tokio::test]
async fn removal_preview_is_read_only_and_binds_current_inode_and_receipt_facts() {
    let registry = Registry::start(|_, _| {}).await;
    let parent = tempfile::tempdir().unwrap();
    let root = crate::oci::tests::private_dir();
    let owner = ImageCache::at(root.path()).unwrap();
    let puller = registry.puller().with_cache(&owner);
    let image = puller.pull(&registry.reference(), parent.path()).await.unwrap();
    let key = image.cache_identity().key();
    drop(image);
    registry.task.abort();
    let first = owner.preview_removal(&key).await.unwrap();
    assert!(first.allowed());
    assert_eq!(first.key(), &key);
    assert_eq!(first.token(), owner.preview_removal(&key).await.unwrap().token());
    let receipt_path = root.path().join("blobs").join(format!("receipt-{}", key.as_str()));
    let receipt = puller.cached_receipt(&key).await.unwrap().unwrap();
    let scoped = owner
        .inner
        .for_repository(&image_reference(&registry.reference()).unwrap());
    let expected = receipt
        .blobs()
        .iter()
        .map(|blob| {
            std::fs::metadata(root.path().join("blobs").join(scoped.entry_name(&blob.digest).unwrap()))
                .unwrap()
                .blocks()
                * 512
        })
        .sum::<u64>()
        + std::fs::metadata(&receipt_path).unwrap().blocks() * 512;
    assert_eq!(first.reclaimable_bytes(), expected);
    let layer = root.path().join("blobs").join(
        scoped
            .entry_name(&super::super::super::tests::digest(&registry.layer))
            .unwrap(),
    );
    std::fs::write(&layer, vec![b'x'; registry.layer.len()]).unwrap();
    let changed = owner.preview_removal(&key).await.unwrap();
    assert_ne!(first.token(), changed.token());
    let bytes = std::fs::read(&receipt_path).unwrap();
    std::fs::write(&receipt_path, &bytes).unwrap();
    assert_ne!(changed.token(), owner.preview_removal(&key).await.unwrap().token());
    assert_eq!(std::fs::read(&receipt_path).unwrap(), bytes);
    assert!(crate::oci::CacheKey::parse("../../outside").is_err());
    let missing = crate::oci::CacheIdentity::new(
        &format!("localhost/missing@sha256:{}", "f".repeat(64)),
        "arm64",
        RUNTIME_CONTRACT,
    )
    .unwrap()
    .key();
    assert!(owner.preview_removal(&missing).await.is_err());
}

#[tokio::test]
async fn removal_preview_refuses_materializing_layouts_and_retained_root_links() {
    use capsem_foundation::unix::lock::{try_acquire_existing, LockAttempt, LockMode};
    let registry = Registry::start(|_, _| {}).await;
    let parent = tempfile::tempdir().unwrap();
    let root = crate::oci::tests::private_dir();
    let owner = ImageCache::at(root.path()).unwrap();
    let puller = registry.puller().with_cache(&owner);
    let image = puller.pull(&registry.reference(), parent.path()).await.unwrap();
    let key = image.cache_identity().key();
    let barrier = root.path().join("locks/materialization.lock");
    assert!(matches!(
        try_acquire_existing(&barrier, LockMode::Exclusive).unwrap(),
        LockAttempt::Contended
    ));
    assert!(!owner.preview_removal(&key).await.unwrap().allowed());
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
    drop(image);
    assert!(
        matches!(
            try_acquire_existing(&barrier, LockMode::Exclusive).unwrap(),
            LockAttempt::Acquired(_)
        ),
        "no new materialization can start while removal holds this admission barrier"
    );
    registry.task.abort();
    let protected = owner.preview_removal(&key).await.unwrap();
    assert!(!protected.allowed());
    assert_eq!(protected.protected_roots(), 1);
    drop(filesystem);
    assert!(owner.preview_removal(&key).await.unwrap().allowed());
}
