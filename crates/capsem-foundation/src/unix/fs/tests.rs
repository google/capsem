use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
use std::sync::{Arc, Barrier};

use super::{atomic_write_private, ensure_private_dir, filesystem_space};

#[test]
fn filesystem_capacity_is_internally_consistent() {
    let root = tempfile::tempdir().unwrap();
    let space = filesystem_space(root.path()).unwrap();
    assert!(space.total_bytes > 0);
    assert!(space.free_bytes <= space.total_bytes);
    assert!(space.available_bytes <= space.free_bytes);
}

#[test]
fn private_directory_is_created_owner_only() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("private");
    ensure_private_dir(&path).unwrap();

    let metadata = std::fs::symlink_metadata(path).unwrap();
    assert!(metadata.is_dir());
    assert_eq!(metadata.uid(), nix::unistd::getuid().as_raw());
    assert_eq!(metadata.permissions().mode() & 0o777, 0o700);
}

#[test]
fn private_directory_refuses_symlinks_and_permissive_modes() {
    let root = tempfile::tempdir().unwrap();
    let elsewhere = root.path().join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    let link = root.path().join("link");
    symlink(&elsewhere, &link).unwrap();
    assert!(ensure_private_dir(&link).is_err());

    let permissive = root.path().join("permissive");
    std::fs::create_dir(&permissive).unwrap();
    std::fs::set_permissions(&permissive, std::fs::Permissions::from_mode(0o750)).unwrap();
    assert!(ensure_private_dir(&permissive).is_err());

    let unusable = root.path().join("unusable");
    std::fs::create_dir(&unusable).unwrap();
    std::fs::set_permissions(&unusable, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(ensure_private_dir(&unusable).is_err());
}

#[test]
fn private_atomic_write_replaces_content_at_mode_600() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("secret");
    std::fs::write(&path, b"old").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();

    atomic_write_private(&path, b"new secret").unwrap();

    assert_eq!(std::fs::read(&path).unwrap(), b"new secret");
    assert_eq!(
        std::fs::symlink_metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn concurrent_private_writes_never_publish_partial_content() {
    let root = tempfile::tempdir().unwrap();
    let path = Arc::new(root.path().join("secret"));
    let barrier = Arc::new(Barrier::new(3));
    let first = vec![b'a'; 32 * 1024];
    let second = vec![b'b'; 48 * 1024];
    let handles = [first.clone(), second.clone()].map(|content| {
        let path = Arc::clone(&path);
        let barrier = Arc::clone(&barrier);
        std::thread::spawn(move || {
            barrier.wait();
            atomic_write_private(&path, &content).unwrap();
        })
    });
    barrier.wait();
    for handle in handles {
        handle.join().unwrap();
    }

    let published = std::fs::read(&*path).unwrap();
    assert!(published == first || published == second);
    assert_eq!(root.path().read_dir().unwrap().count(), 1);
}
