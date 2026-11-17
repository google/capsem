use crate::unix::{
    contained::ContainedDir,
    fs::{atomic_write_private_at, ensure_private_dir_restrict_existing},
};
use std::io::Read;
use std::{ffi::OsStr, os::unix::fs::PermissionsExt};

fn private_directory() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    ensure_private_dir_restrict_existing(root.path()).unwrap();
    root
}

#[test]
fn contained_atomic_write_publishes_complete_private_bytes_and_replaces_symlink_itself() {
    let root = private_directory();
    let directory = ContainedDir::open_root(root.path()).unwrap();
    let name = OsStr::new("environment.toml");
    atomic_write_private_at(&directory, name, b"first complete document").unwrap();
    let mut held = directory
        .open_file(name, crate::unix::contained::ContainedOpenOptions::read_only())
        .unwrap();
    atomic_write_private_at(&directory, name, b"second complete document").unwrap();
    let mut old = Vec::new();
    held.read_to_end(&mut old).unwrap();
    assert_eq!(old, b"first complete document");
    let target = root.path().join("environment.toml");
    assert_eq!(std::fs::read(&target).unwrap(), b"second complete document");
    assert_eq!(std::fs::metadata(&target).unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    let outside = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(outside.path(), b"outside stays intact").unwrap();
    std::fs::remove_file(&target).unwrap();
    std::os::unix::fs::symlink(outside.path(), &target).unwrap();
    atomic_write_private_at(&directory, name, b"new complete document").unwrap();
    assert!(!std::fs::symlink_metadata(&target).unwrap().is_symlink());
    assert_eq!(std::fs::read(outside.path()).unwrap(), b"outside stays intact");
    assert_eq!(std::fs::read(target).unwrap(), b"new complete document");
}

#[test]
fn contained_atomic_write_retains_open_directory_identity_after_path_replacement() {
    let root = private_directory();
    let active = root.path().join("active");
    ensure_private_dir_restrict_existing(&active).unwrap();
    let directory = ContainedDir::open_root(&active).unwrap();
    let moved = root.path().join("original");
    std::fs::rename(&active, &moved).unwrap();
    ensure_private_dir_restrict_existing(&active).unwrap();
    std::fs::write(active.join("environment.toml"), b"replacement stays intact").unwrap();
    atomic_write_private_at(
        &directory,
        OsStr::new("environment.toml"),
        b"held directory publication",
    )
    .unwrap();
    assert_eq!(
        std::fs::read(moved.join("environment.toml")).unwrap(),
        b"held directory publication"
    );
    assert_eq!(
        std::fs::read(active.join("environment.toml")).unwrap(),
        b"replacement stays intact"
    );
}

#[test]
fn contained_atomic_write_failure_and_invalid_components_leave_no_temporary_files() {
    let root = private_directory();
    let directory = ContainedDir::open_root(root.path()).unwrap();
    std::fs::create_dir(root.path().join("destination")).unwrap();
    assert!(atomic_write_private_at(&directory, OsStr::new("destination"), b"cannot replace a directory").is_err());
    for name in ["", ".", "..", "../escape", "/escape"] {
        assert!(atomic_write_private_at(&directory, OsStr::new(name), b"invalid destination").is_err());
    }
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    assert!(root.path().join("destination").is_dir());
}
