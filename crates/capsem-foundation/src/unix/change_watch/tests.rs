use super::*;
use crate::unix::contained::ContainedDir;
use std::{
    fs::{File, OpenOptions},
    io::Write,
    os::fd::AsFd,
    os::unix::fs::PermissionsExt,
};

#[test]
fn directory_path_watch_tracks_ancestor_replacement_and_retains_original_root() {
    use std::os::unix::fs::MetadataExt;
    let parent = tempfile::tempdir().unwrap();
    let ancestor = parent.path().join("ancestor");
    let path = ancestor.join("cache");
    std::fs::create_dir_all(&path).unwrap();
    let mut watch = ChangeWatch::new().unwrap();
    let root = watch.open_directory(&path).unwrap();
    let original = root.metadata().unwrap().ino();
    assert!(!watch.changed().unwrap());
    std::fs::write(parent.path().join("unrelated"), b"sibling").unwrap();
    assert!(
        !watch.changed().unwrap(),
        "unrelated ancestor siblings must not invalidate"
    );
    std::fs::rename(&ancestor, parent.path().join("moved")).unwrap();
    std::fs::create_dir_all(&path).unwrap();
    assert!(
        watch.changed().unwrap(),
        "the root inode did not move, but its ancestor binding changed"
    );
    assert_eq!(root.metadata().unwrap().ino(), original);
    assert_ne!(
        ContainedDir::open_root(&path).unwrap().metadata().unwrap().ino(),
        original
    );
}

#[test]
fn directory_path_watch_tracks_alias_rebinding_without_redirecting_held_descriptors() {
    use std::os::unix::fs::MetadataExt;
    let parent = tempfile::tempdir().unwrap();
    let original = parent.path().join("original");
    let replacement = parent.path().join("replacement");
    std::fs::create_dir_all(original.join("cache")).unwrap();
    std::fs::create_dir_all(replacement.join("cache")).unwrap();
    let alias = parent.path().join("alias");
    std::os::unix::fs::symlink(&original, &alias).unwrap();
    let mut watch = ChangeWatch::new().unwrap();
    let held = watch.open_directory(&alias.join("cache")).unwrap();
    let identity = held.metadata().unwrap().ino();
    assert!(!watch.changed().unwrap());
    std::fs::remove_file(&alias).unwrap();
    std::os::unix::fs::symlink(&replacement, &alias).unwrap();
    assert!(watch.changed().unwrap());
    assert_eq!(held.metadata().unwrap().ino(), identity);
    assert_ne!(
        ContainedDir::open_root(&alias.join("cache"))
            .unwrap()
            .metadata()
            .unwrap()
            .ino(),
        identity
    );
}

#[test]
fn directory_path_watch_refuses_relative_or_parent_components() {
    for path in [std::path::Path::new("relative"), std::path::Path::new("/tmp/../tmp")] {
        let mut watch = ChangeWatch::new().unwrap();
        assert!(watch.open_directory(path).is_err());
        assert!(watch.changed().unwrap(), "registration failure must stay conservative");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn ancestor_notification_backlog_cannot_hide_rebinding() {
    let parent = tempfile::tempdir().unwrap();
    let ancestor = parent.path().join("ancestor");
    let path = ancestor.join("cache");
    std::fs::create_dir_all(&path).unwrap();
    let mut watch = ChangeWatch::new().unwrap();
    let _held = watch.open_directory(&path).unwrap();
    for index in 0..200 {
        std::fs::write(parent.path().join(format!("unrelated-{index}")), b"sibling").unwrap();
    }
    std::fs::rename(&ancestor, parent.path().join("moved")).unwrap();
    assert!(
        watch.changed().unwrap(),
        "excess ignored siblings must not hide a queued binding change"
    );
    assert!(watch.changed().unwrap());
}

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
