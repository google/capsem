use super::super::tests::{digest, Registry};
use super::*;

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
