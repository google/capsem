use super::*;
use crate::unix::contained::ContainedDir;
use std::{
    fs::{File, OpenOptions},
    io::Write,
    os::fd::AsFd,
    os::unix::fs::PermissionsExt,
};

#[test]
fn read_only_access_is_quiet_but_hardlink_writes_are_sticky() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("payload");
    let alias = root.path().join("alias");
    std::fs::write(&path, b"original").unwrap();
    std::fs::hard_link(&path, &alias).unwrap();
    let file = File::open(&path).unwrap();
    let mut watch = ChangeWatch::new().unwrap();
    watch.add(file.as_fd()).unwrap();
    drop(file);
    assert!(!watch.changed().unwrap());
    assert_eq!(std::fs::read(&path).unwrap(), b"original");
    assert!(!watch.changed().unwrap(), "readonly access must not invalidate");
    OpenOptions::new()
        .write(true)
        .open(&alias)
        .unwrap()
        .write_all(b"modified")
        .unwrap();
    assert!(
        watch.changed().unwrap(),
        "writes through another hardlink affect the watched inode"
    );
    assert!(watch.changed().unwrap(), "a consumed event must never restore validity");
}

#[test]
fn namespace_and_attribute_changes_invalidate_opened_watches() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("payload");
    std::fs::write(&path, b"original").unwrap();
    let file = File::open(&path).unwrap();
    let mut attributes = ChangeWatch::new().unwrap();
    attributes.add(file.as_fd()).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();
    assert!(attributes.changed().unwrap());
    let directory = ContainedDir::open_root(root.path()).unwrap();
    let mut names = ChangeWatch::new().unwrap();
    names.add(directory.as_fd()).unwrap();
    std::fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink("elsewhere", &path).unwrap();
    assert!(
        names.changed().unwrap(),
        "replacement must invalidate even while the old inode is open"
    );
}

#[test]
fn directory_watch_stays_bound_after_path_rename() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("cache");
    std::fs::create_dir(&path).unwrap();
    let directory = ContainedDir::open_root(&path).unwrap();
    let mut watch = ChangeWatch::new().unwrap();
    watch.add(directory.as_fd()).unwrap();
    drop(directory);
    std::fs::rename(&path, root.path().join("moved")).unwrap();
    assert!(watch.changed().unwrap());
}
