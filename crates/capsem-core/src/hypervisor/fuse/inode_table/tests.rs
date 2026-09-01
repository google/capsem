use super::*;
use std::path::PathBuf;

fn temp_share(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("capsem-fuse-test").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

// Inode table operations

#[test]
fn root_is_1() {
    let dir = temp_share("inode-root");
    let table = InodeTable::new(&dir).unwrap();
    assert!(table.get(1).is_some());
}

#[test]
fn lookup_creates_inode() {
    let dir = temp_share("inode-lookup");
    std::fs::write(dir.join("hello.txt"), b"world").unwrap();
    let mut table = InodeTable::new(&dir).unwrap();
    let ino = table.lookup(1, b"hello.txt").unwrap();
    assert!(ino >= 2);
    assert!(table.get(ino).is_some());
}

#[test]
fn lookup_same_name_same_inode() {
    let dir = temp_share("inode-same");
    std::fs::write(dir.join("file.txt"), b"data").unwrap();
    let mut table = InodeTable::new(&dir).unwrap();
    let ino1 = table.lookup(1, b"file.txt").unwrap();
    let ino2 = table.lookup(1, b"file.txt").unwrap();
    assert_eq!(ino1, ino2);
}

#[test]
fn lookup_preserves_symlink_with_absolute_target() {
    let dir = temp_share("inode-symlink-absolute");
    std::os::unix::fs::symlink("/etc/passwd", dir.join("link")).unwrap();
    let mut table = InodeTable::new(&dir).unwrap();

    let ino = table.lookup(1, b"link").unwrap();

    assert_eq!(table.get(ino).unwrap(), &dir.join("link"));
}

#[test]
fn lookup_preserves_broken_symlink() {
    let dir = temp_share("inode-symlink-broken");
    std::os::unix::fs::symlink("missing-target", dir.join("link")).unwrap();
    let mut table = InodeTable::new(&dir).unwrap();

    let ino = table.lookup(1, b"link").unwrap();

    assert_eq!(table.get(ino).unwrap(), &dir.join("link"));
}

#[test]
fn lookup_bumps_refcount() {
    let dir = temp_share("inode-refcount");
    std::fs::write(dir.join("file.txt"), b"data").unwrap();
    let mut table = InodeTable::new(&dir).unwrap();
    let ino = table.lookup(1, b"file.txt").unwrap();
    table.lookup(1, b"file.txt").unwrap();
    table.forget(ino, 1);
    assert!(table.get(ino).is_some());
    table.forget(ino, 1);
    assert!(table.get(ino).is_none());
}

#[test]
fn forget_removes_at_zero() {
    let dir = temp_share("inode-forget");
    std::fs::write(dir.join("file.txt"), b"data").unwrap();
    let mut table = InodeTable::new(&dir).unwrap();
    let ino = table.lookup(1, b"file.txt").unwrap();
    table.forget(ino, 1);
    assert!(table.get(ino).is_none());
}

#[test]
fn forget_root_noop() {
    let dir = temp_share("inode-forget-root");
    let mut table = InodeTable::new(&dir).unwrap();
    table.forget(1, u64::MAX);
    assert!(table.get(1).is_some());
}

#[test]
fn forget_saturates() {
    let dir = temp_share("inode-saturate");
    std::fs::write(dir.join("file.txt"), b"data").unwrap();
    let mut table = InodeTable::new(&dir).unwrap();
    let ino = table.lookup(1, b"file.txt").unwrap();
    table.forget(ino, 100);
    assert!(table.get(ino).is_none());
}

#[test]
fn nonexistent_returns_none() {
    let dir = temp_share("inode-noent");
    let mut table = InodeTable::new(&dir).unwrap();
    assert!(table.lookup(1, b"nonexistent.txt").is_none());
}

// Path traversal security (adversarial)

#[test]
fn rejects_dotdot() {
    let dir = temp_share("path-dotdot");
    assert!(InodeTable::new(&dir).unwrap().lookup(1, b"..").is_none());
}

#[test]
fn rejects_dot() {
    let dir = temp_share("path-dot");
    assert!(InodeTable::new(&dir).unwrap().lookup(1, b".").is_none());
}

#[test]
fn rejects_slash() {
    let dir = temp_share("path-slash");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("sub/file.txt"), b"data").unwrap();
    assert!(InodeTable::new(&dir).unwrap().lookup(1, b"sub/file.txt").is_none());
}

#[test]
fn rejects_null() {
    let dir = temp_share("path-null");
    assert!(InodeTable::new(&dir).unwrap().lookup(1, b"file\0.txt").is_none());
}

#[test]
fn rejects_empty() {
    let dir = temp_share("path-empty");
    assert!(InodeTable::new(&dir).unwrap().lookup(1, b"").is_none());
}

#[test]
fn preserves_absolute_symlink_target_without_following_it() {
    let dir = temp_share("path-symlink-escape");
    std::os::unix::fs::symlink("/etc/passwd", dir.join("escape")).unwrap();
    let mut table = InodeTable::new(&dir).unwrap();

    let ino = table.lookup(1, b"escape").unwrap();

    assert_eq!(table.get(ino).unwrap(), &dir.join("escape"));
}

#[test]
fn preserves_symlink_to_directory_outside_share_without_following_it() {
    let dir = temp_share("path-chain-escape");
    std::os::unix::fs::symlink("/tmp", dir.join("link")).unwrap();
    let mut table = InodeTable::new(&dir).unwrap();

    let ino = table.lookup(1, b"link").unwrap();

    assert_eq!(table.get(ino).unwrap(), &dir.join("link"));
}

#[test]
fn allows_regular_file() {
    let dir = temp_share("path-regular");
    std::fs::write(dir.join("ok.txt"), b"fine").unwrap();
    assert!(InodeTable::new(&dir).unwrap().lookup(1, b"ok.txt").is_some());
}

#[test]
fn allows_dotfile() {
    let dir = temp_share("path-dotfile");
    std::fs::write(dir.join(".hidden"), b"secret").unwrap();
    assert!(InodeTable::new(&dir).unwrap().lookup(1, b".hidden").is_some());
}

#[test]
fn allows_subdirectory_traversal() {
    let dir = temp_share("path-subdir");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("sub/file.txt"), b"data").unwrap();
    let mut table = InodeTable::new(&dir).unwrap();
    let sub_ino = table.lookup(1, b"sub").unwrap();
    assert!(table.lookup(sub_ino, b"file.txt").is_some());
}
