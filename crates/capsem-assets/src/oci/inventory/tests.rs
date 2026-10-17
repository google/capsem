use std::{
    fs::File,
    io::{Seek, SeekFrom, Write},
    os::unix::fs::{symlink, MetadataExt},
};

use super::*;

#[tokio::test]
async fn cache_owner_usage_includes_the_live_mutation_lock() {
    let root = super::super::tests::private_dir();
    let cache = super::super::cache::BlobCache::at(root.path()).unwrap();
    cache.prepare().await.unwrap();
    std::fs::write(root.path().join("locks/01.lock"), b"live control").unwrap();
    std::fs::write(root.path().join("blobs/.partial-download"), b"partial bytes").unwrap();
    let observed = cache.usage().await.unwrap();
    let expected = measure(&ContainedDir::open_root(root.path()).unwrap()).unwrap();
    assert_eq!(observed, expected);
    assert_eq!(
        observed.entries, 6,
        "root, blob/lock directories, partial, stripe and mutation lock"
    );
}

#[test]
fn sparse_and_hardlinked_bytes_include_controls_and_partials_once() {
    let root = tempfile::tempdir().unwrap();
    let blobs = root.path().join("blobs");
    let locks = root.path().join("locks");
    std::fs::create_dir(&blobs).unwrap();
    std::fs::create_dir(&locks).unwrap();
    let payload = blobs.join("immutable-payload");
    let mut file = File::create(&payload).unwrap();
    file.seek(SeekFrom::Start(64 * 1024 * 1024)).unwrap();
    file.write_all(b"root bytes").unwrap();
    file.sync_all().unwrap();
    std::fs::hard_link(&payload, blobs.join("duplicate")).unwrap();
    std::fs::hard_link(&payload, locks.join("duplicate-across-directory")).unwrap();
    let controls = [
        blobs.join("receipt-control"),
        blobs.join(".partial-download"),
        locks.join("01.lock"),
        root.path().join("mutation.lock"),
    ];
    for path in &controls {
        std::fs::write(path, b"control bytes").unwrap();
    }
    let unique = [root.path().to_owned(), blobs, locks, payload];
    let expected: u64 = unique
        .iter()
        .chain(&controls)
        .map(|path| std::fs::symlink_metadata(path).unwrap().blocks() * 512)
        .sum();
    let usage = measure(&ContainedDir::open_root(root.path()).unwrap()).unwrap();
    assert_eq!(usage.allocated_bytes, expected);
    assert_eq!(usage.entries, 10);
    assert_eq!(usage.unique_inodes, 8);
    assert!(usage.allocated_bytes < 64 * 1024 * 1024);
}

#[test]
fn symlinks_count_their_own_storage_without_visiting_targets() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret"), vec![7; 1024 * 1024]).unwrap();
    let link = root.path().join("outside");
    symlink(outside.path(), &link).unwrap();
    let expected = [root.path(), link.as_path()]
        .iter()
        .map(|path| std::fs::symlink_metadata(path).unwrap().blocks() * 512)
        .sum::<u64>();
    let usage = measure(&ContainedDir::open_root(root.path()).unwrap()).unwrap();
    assert_eq!(usage.allocated_bytes, expected);
    assert_eq!(usage.entries, 2);
    assert_eq!(usage.unique_inodes, 2);
}
