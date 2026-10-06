use super::super::tests::{digest, Registry};
use super::*;

#[tokio::test]
async fn verified_legacy_cache_materialization_publishes_its_missing_receipt_offline() {
    let registry = Registry::start(|_, _| {}).await;
    let parent = tempfile::tempdir().unwrap();
    let root = super::super::super::tests::private_dir();
    let mut puller = registry.puller();
    puller.cache = Some(BlobCache::at(root.path()).unwrap());
    let image = puller.pull(&registry.reference(), parent.path()).await.unwrap();
    let key = image.cache_identity().key();
    std::fs::remove_file(root.path().join("blobs").join(format!("receipt-{}", key.as_str()))).unwrap();
    let reference = registry
        .reference()
        .replace(":latest", &format!("@{}", image.image_digest));
    registry.task.abort();
    let local = puller.pull_cached(&reference, parent.path()).await.unwrap();
    assert_eq!(local.cache_identity(), image.cache_identity());
    assert!(
        puller.cached_receipt(&key).await.unwrap().is_some(),
        "verified pre-receipt cache data needs owner metadata too"
    );
}

#[tokio::test]
async fn published_filesystem_receipts_bind_the_exact_native_manifest_descriptor() {
    let registry = Registry::start(|_, _| {}).await;
    let parent = tempfile::tempdir().unwrap();
    let root = super::super::super::tests::private_dir();
    let mut puller = registry.puller();
    puller.cache = Some(BlobCache::at(root.path()).unwrap());
    let image = puller.pull(&registry.reference(), parent.path()).await.unwrap();
    let key = image.cache_identity().key();
    let original = puller.cached_receipt(&key).await.unwrap().unwrap();
    let wrong_reference = registry.published_filesystem(&image.source_digest, Some(1));
    let subject = ContentDigest::parse(&image.source_digest).unwrap();
    let wrong = puller
        .fetch_rootfs(&wrong_reference, &subject, parent.path())
        .await
        .unwrap();
    assert!(
        puller.retain_cached_root(&key, &wrong).await.is_err(),
        "filesystem receipt must bind native subject size as well as digest"
    );
    assert_eq!(
        puller
            .cached_receipt(&key)
            .await
            .unwrap()
            .unwrap()
            .generation()
            .unwrap(),
        original.generation().unwrap()
    );
    let reference = registry.published_filesystem(&image.source_digest, None);
    let filesystem = puller.fetch_rootfs(&reference, &subject, parent.path()).await.unwrap();
    let namespace = puller
        .cache
        .as_ref()
        .unwrap()
        .for_repository(&image_reference(&registry.reference()).unwrap());
    let config = root
        .path()
        .join("blobs")
        .join(namespace.entry_name(&digest(&registry.config)).unwrap());
    std::fs::write(&config, vec![b'x'; registry.config.len()]).unwrap();
    assert!(
        puller.retain_cached_root(&key, &filesystem).await.is_err(),
        "a failed byte recheck must not publish an updated receipt"
    );
    assert_eq!(
        puller
            .cached_receipt(&key)
            .await
            .unwrap()
            .unwrap()
            .generation()
            .unwrap(),
        original.generation().unwrap()
    );
    std::fs::write(&config, &registry.config).unwrap();
    puller.retain_cached_root(&key, &filesystem).await.unwrap();
    let retained = puller.cached_receipt(&key).await.unwrap().unwrap();
    assert!(retained.root().is_some());
    assert_ne!(retained.generation().unwrap(), original.generation().unwrap());
    registry.task.abort();
    let fresh = puller.cached_receipt(&key).await.unwrap().unwrap();
    assert_eq!(fresh.generation().unwrap(), retained.generation().unwrap());
}

#[tokio::test]
async fn successful_pull_retains_a_durable_receipt_without_registry_secrets_or_ready_claims() {
    let registry = Registry::start(|_, _| {}).await;
    let parent = tempfile::tempdir().unwrap();
    let root = super::super::super::tests::private_dir();
    let cache = BlobCache::at(root.path()).unwrap();
    let mut puller = registry.puller();
    puller.authentication = RegistryAuth::Basic("test-account".into(), "private-registry-secret".into());
    puller.cache = Some(cache.clone());
    let image = puller.pull(&registry.reference(), parent.path()).await.unwrap();
    let key = image.cache_identity().key();
    let receipt = puller.cached_receipt(&key).await.unwrap().unwrap();
    assert_eq!(receipt.identity(), image.cache_identity());
    assert_eq!(receipt.native_digest().as_str(), image.source_digest);
    assert!(receipt.verified_at_unix_ns() > 0);
    let bytes = receipt.encode().unwrap();
    let document: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(
        document.get("ready").is_none(),
        "loaded metadata must not declare readiness"
    );
    assert!(!String::from_utf8(bytes).unwrap().contains("private-registry-secret"));
    let mut fresh = registry.puller();
    fresh.cache = Some(cache);
    assert_eq!(
        fresh.cached_receipt(&key).await.unwrap().unwrap().generation().unwrap(),
        receipt.generation().unwrap()
    );
    let path = root.path().join("blobs").join(format!("receipt-{}", key.as_str()));
    let valid = std::fs::read(&path).unwrap();
    let target = parent.path().join("foreign-receipt");
    std::fs::write(&target, &valid).unwrap();
    std::fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(&target, &path).unwrap();
    assert!(
        fresh.cached_receipt(&key).await.is_err(),
        "receipt reader must not follow links"
    );
    assert_eq!(std::fs::read(&target).unwrap(), valid);
    std::fs::remove_file(&path).unwrap();
    std::fs::write(&path, &valid).unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len((METADATA_LIMIT + 1) as u64)
        .unwrap();
    assert!(
        fresh.cached_receipt(&key).await.is_err(),
        "oversized disk records must be bounded before allocation"
    );
    std::fs::write(&path, b"{\"schema_version\":999}").unwrap();
    assert!(fresh.cached_receipt(&key).await.is_err());
    std::fs::remove_file(&path).unwrap();
    assert!(fresh.cached_receipt(&key).await.unwrap().is_none());
}

#[tokio::test]
async fn cache_preferred_images_fetch_cold_pins_refresh_tags_and_work_without_registry() {
    let registry = Registry::start(|_, _| {}).await;
    let parent = tempfile::tempdir().unwrap();
    let original = registry
        .puller()
        .pull(&registry.reference(), parent.path())
        .await
        .unwrap();
    let pin = registry
        .reference()
        .replace(":latest", &format!("@{}", original.source_digest));
    let root = super::super::super::tests::private_dir();
    let mut puller = registry.puller();
    puller.cache = Some(BlobCache::at(root.path()).unwrap());
    let cold = puller.pull_prefer_cached(&pin, parent.path()).await.unwrap();
    assert_eq!(cold.source_digest, original.source_digest);
    let before = registry.requests.lock().unwrap().len();
    let fresh = puller
        .pull_prefer_cached(&registry.reference(), parent.path())
        .await
        .unwrap();
    assert_eq!(fresh.source_digest, original.source_digest);
    assert!(
        registry.requests.lock().unwrap().len() > before,
        "moving tags must be resolved again"
    );
    registry.task.abort();
    let local = puller.pull_prefer_cached(&pin, parent.path()).await.unwrap();
    assert_eq!(local.files(), cold.files());
    for file in local.files() {
        assert_eq!(
            std::fs::read(local.path().join(file)).unwrap(),
            std::fs::read(cold.path().join(file)).unwrap()
        );
    }
}

#[tokio::test]
async fn cache_preferred_published_roots_fetch_cold_and_preserve_warm_inodes_offline() {
    use super::super::tests::{rootfs_reference, rootfs_registry, rootfs_subject};
    use std::os::unix::fs::MetadataExt;
    let registry = rootfs_registry(|_, _| {}).await;
    let parent = tempfile::tempdir().unwrap();
    let root = super::super::super::tests::private_dir();
    let mut puller = registry.puller();
    puller.cache = Some(BlobCache::at(root.path()).unwrap());
    let reference = rootfs_reference(&registry);
    let cold = puller
        .fetch_rootfs_prefer_cached(&reference, &rootfs_subject(), parent.path())
        .await
        .unwrap();
    registry.task.abort();
    let warm = puller
        .fetch_rootfs_prefer_cached(&reference, &rootfs_subject(), parent.path())
        .await
        .unwrap();
    assert_eq!(
        std::fs::metadata(cold.path()).unwrap().ino(),
        std::fs::metadata(warm.path()).unwrap().ino()
    );
    assert_eq!(cold.digest(), warm.digest());
}

#[tokio::test]
async fn cached_published_root_preserves_the_verified_inode_without_registry_access() {
    use super::super::tests::{rootfs_reference, rootfs_registry, rootfs_subject, ROOTFS_BYTES};
    use std::os::unix::fs::MetadataExt;
    let registry = rootfs_registry(|_, _| {}).await;
    let parent = tempfile::tempdir().unwrap();
    let root = super::super::super::tests::private_dir();
    let cache = BlobCache::at(root.path()).unwrap();
    let mut puller = registry.puller();
    puller.cache = Some(cache.clone());
    let reference = rootfs_reference(&registry);
    let online = puller
        .fetch_rootfs(&reference, &rootfs_subject(), parent.path())
        .await
        .unwrap();
    registry.task.abort();
    let local = puller
        .fetch_rootfs_cached(&reference, &rootfs_subject(), parent.path())
        .await
        .unwrap();
    assert_eq!(local.digest(), online.digest());
    assert_eq!(std::fs::read(local.path()).unwrap(), ROOTFS_BYTES);
    assert_eq!(
        std::fs::metadata(local.path()).unwrap().ino(),
        std::fs::metadata(online.path()).unwrap().ino()
    );
    assert_eq!(std::fs::metadata(local.path()).unwrap().mode() & 0o777, 0o444);
    let wrong = ContentDigest::parse(&format!("sha256:{}", "f".repeat(64))).unwrap();
    assert!(puller
        .fetch_rootfs_cached(&reference, &wrong, parent.path())
        .await
        .is_err());
    let name = cache
        .for_repository(&image_reference(&reference).unwrap())
        .entry_name(local.digest().as_str())
        .unwrap();
    std::fs::remove_file(root.path().join("blobs").join(format!("immutable-{name}"))).unwrap();
    assert!(puller
        .fetch_rootfs_cached(&reference, &rootfs_subject(), parent.path())
        .await
        .is_err());
    assert_eq!(std::fs::read(online.path()).unwrap(), ROOTFS_BYTES);
    drop(local);
    drop(online);
    assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn cached_pinned_image_materializes_without_a_registry_and_rehashes_every_blob() {
    let registry = Registry::start(|_, _| {}).await;
    let parent = tempfile::tempdir().unwrap();
    let root = super::super::super::tests::private_dir();
    let cache = BlobCache::at(root.path()).unwrap();
    let mut online = registry.puller();
    online.cache = Some(cache.clone());
    let original = online.pull(&registry.reference(), parent.path()).await.unwrap();
    let reference = registry
        .reference()
        .replace(":latest", &format!("@{}", original.image_digest));
    let mut offline = registry.puller();
    offline.cache = Some(cache.clone());
    registry.task.abort();
    let image = offline.pull_cached(&reference, parent.path()).await.unwrap();
    let mut wrong_architecture =
        Puller::configured("amd64", RegistryAuth::Anonymous, ClientProtocol::Http, None).unwrap();
    wrong_architecture.cache = Some(cache.clone());
    let native_reference = reference.replace(&original.image_digest, &original.source_digest);
    let error = wrong_architecture
        .pull_cached(&native_reference, parent.path())
        .await
        .err()
        .unwrap();
    assert!(format!("{error:#}").contains("linux/amd64"));
    assert_eq!(image.image_digest, original.image_digest);
    assert_eq!(image.cache_identity(), original.cache_identity());
    assert_eq!(image.cache_identity().image().digest().as_str(), image.image_digest);
    assert_eq!(image.cache_identity().architecture(), "arm64");
    assert_eq!(image.cache_identity().runtime_contract(), RUNTIME_CONTRACT);
    assert_eq!(image.source_digest, original.source_digest);
    assert_eq!(image.files(), original.files());
    for file in image.files() {
        assert_eq!(
            std::fs::read(image.path().join(file)).unwrap(),
            std::fs::read(original.path().join(file)).unwrap()
        );
    }
    drop(image);
    drop(original);
    let namespace = cache.for_repository(&image_reference(&reference).unwrap());
    let name = namespace.entry_name(&digest(&registry.layer)).unwrap();
    let blob = root.path().join("blobs").join(name);
    std::fs::write(&blob, vec![b'x'; registry.layer.len()]).unwrap();
    assert!(offline.pull_cached(&reference, parent.path()).await.is_err());
    std::fs::remove_file(blob).unwrap();
    assert!(offline.pull_cached(&reference, parent.path()).await.is_err());
    let config = namespace.entry_name(&digest(&registry.config)).unwrap();
    std::fs::write(
        root.path().join("blobs").join(config),
        vec![b'x'; registry.config.len()],
    )
    .unwrap();
    assert!(offline.pull_cached(&reference, parent.path()).await.is_err());
    assert!(offline.pull_cached(&registry.reference(), parent.path()).await.is_err());
    assert!(offline
        .pull_cached(&reference.replace("team/image", "another/image"), parent.path())
        .await
        .is_err());
    assert_eq!(
        std::fs::read_dir(parent.path()).unwrap().count(),
        0,
        "failed offline layouts must be reclaimed"
    );
}

#[tokio::test]
async fn cached_manifest_corruption_or_missing_bytes_never_fall_back_to_the_registry() {
    let registry = Registry::start(|_, _| {}).await;
    let parent = tempfile::tempdir().unwrap();
    let root = super::super::super::tests::private_dir();
    let cache = BlobCache::at(root.path()).unwrap();
    let mut puller = registry.puller();
    puller.cache = Some(cache.clone());
    let image = puller.pull(&registry.reference(), parent.path()).await.unwrap();
    let reference = registry
        .reference()
        .replace(":latest", &format!("@{}", image.image_digest));
    let namespace = cache.for_repository(&image_reference(&reference).unwrap());
    let path = root
        .path()
        .join("blobs")
        .join(namespace.entry_name(&image.image_digest).unwrap());
    let original = std::fs::read(&path).unwrap();
    drop(image);
    let before = registry.requests.lock().unwrap().len();
    std::fs::write(&path, vec![b'x'; original.len()]).unwrap();
    assert!(puller.pull_cached(&reference, parent.path()).await.is_err());
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len((METADATA_LIMIT + 1) as u64)
        .unwrap();
    assert!(puller.pull_cached(&reference, parent.path()).await.is_err());
    std::fs::remove_file(&path).unwrap();
    assert!(puller.pull_cached(&reference, parent.path()).await.is_err());
    assert_eq!(
        registry.requests.lock().unwrap().len(),
        before,
        "offline misses must not authenticate or fetch"
    );
    assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 0);
}
