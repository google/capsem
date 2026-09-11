use super::*;

fn identity(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
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
    cache.policy.warm_size_bytes = 6;
    cache.policy.max_size_bytes = 10;
    cache.prepare().await.unwrap();
    let source = staging.path().join("source");
    let a = identity(b"aaaaaa");
    let b = identity(b"bbbbbb");
    std::fs::write(&source, b"aaaaaa").unwrap();
    cache.publish(&a, &source).await.unwrap();
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
