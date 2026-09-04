use std::ffi::OsStr;
use std::io::{Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::symlink;
use std::path::Path;

use nix::sys::stat::Mode;

use super::*;

struct Tree {
    _temporary: tempfile::TempDir,
    root: ContainedDir,
    root_path: PathBuf,
    outside: PathBuf,
}

fn tree() -> Tree {
    let temporary = tempfile::tempdir().unwrap();
    let root_path = temporary.path().join("workspace");
    let outside = temporary.path().join("outside");
    std::fs::create_dir_all(&root_path).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret"), b"secret").unwrap();
    let root = ContainedDir::open_root(&root_path).unwrap();
    Tree {
        _temporary: temporary,
        root,
        root_path,
        outside,
    }
}

#[test]
fn permission_modes_discard_non_permission_bits_portably() {
    assert_eq!(permission_mode(u32::MAX), permission_mode(0o7777));
}

#[test]
fn traversal_refuses_symlinks_at_every_depth() {
    let tree = tree();
    std::fs::create_dir(tree.root_path.join("real")).unwrap();
    symlink(&tree.outside, tree.root_path.join("outside-link")).unwrap();
    symlink("real", tree.root_path.join("inside-link")).unwrap();

    for relative in ["outside-link", "outside-link/deep", "inside-link"] {
        let error = tree.root.walk(Path::new(relative)).unwrap_err();
        assert!(is_symlink_refusal(&error), "{relative}: {error}");
    }
}

#[test]
fn creating_walk_never_crosses_or_replaces_a_symlink() {
    let tree = tree();
    symlink(&tree.outside, tree.root_path.join("link")).unwrap();

    let error = tree.root.walk_creating(Path::new("link/child"), 0o755).unwrap_err();
    assert!(is_symlink_refusal(&error), "{error}");
    assert!(!tree.outside.join("child").exists());

    let leaf = tree.root.walk_creating(Path::new("safe/deep"), 0o750).unwrap();
    assert_eq!(leaf.path(), tree.root_path.canonicalize().unwrap().join("safe/deep"));
}

#[test]
fn file_open_refuses_existing_and_dangling_symlinks() {
    let tree = tree();
    symlink(tree.outside.join("secret"), tree.root_path.join("existing")).unwrap();
    let absent_target = tree.outside.join("absent");
    symlink(&absent_target, tree.root_path.join("dangling")).unwrap();

    for name in ["existing", "dangling"] {
        let error = tree
            .root
            .open_file(OsStr::new(name), ContainedOpenOptions::write_create_truncate(0o644))
            .unwrap_err();
        assert!(is_symlink_refusal(&error), "{name}: {error}");
    }
    assert!(!absent_target.exists());
}

#[test]
fn regular_files_round_trip_through_constrained_options() {
    let tree = tree();
    let mut writer = tree
        .root
        .open_file(OsStr::new("hello"), ContainedOpenOptions::write_create_truncate(0o640))
        .unwrap();
    writer.write_all(b"world").unwrap();
    drop(writer);

    let mut reader = tree
        .root
        .open_file(OsStr::new("hello"), ContainedOpenOptions::read_only())
        .unwrap();
    let mut contents = String::new();
    reader.read_to_string(&mut contents).unwrap();
    assert_eq!(contents, "world");
}

#[test]
fn special_files_are_refused_without_blocking() {
    let tree = tree();
    nix::unistd::mkfifo(&tree.root_path.join("pipe"), Mode::from_bits_truncate(0o600)).unwrap();

    let error = tree
        .root
        .open_file(OsStr::new("pipe"), ContainedOpenOptions::read_only())
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
}

#[test]
fn entries_classify_but_never_follow_special_files() {
    let tree = tree();
    std::fs::create_dir(tree.root_path.join("directory")).unwrap();
    std::fs::write(tree.root_path.join("file"), b"data").unwrap();
    symlink(&tree.outside, tree.root_path.join("link")).unwrap();

    let kinds: std::collections::HashMap<_, _> = tree
        .root
        .entries()
        .unwrap()
        .into_iter()
        .map(|entry| (entry.name, entry.kind))
        .collect();
    assert_eq!(kinds[OsStr::new("directory")], EntryKind::Directory);
    assert_eq!(kinds[OsStr::new("file")], EntryKind::File);
    assert_eq!(kinds[OsStr::new("link")], EntryKind::Other);
}

#[test]
fn entry_kind_distinguishes_absence_files_and_links() {
    let tree = tree();
    std::fs::write(tree.root_path.join("file"), b"data").unwrap();
    symlink(&tree.outside, tree.root_path.join("link")).unwrap();

    assert_eq!(tree.root.entry_kind(OsStr::new("missing")).unwrap(), None);
    assert_eq!(tree.root.entry_kind(OsStr::new("file")).unwrap(), Some(EntryKind::File));
    assert_eq!(
        tree.root.entry_kind(OsStr::new("link")).unwrap(),
        Some(EntryKind::Other)
    );
}

#[test]
fn invalid_components_are_rejected_before_mutation() {
    let tree = tree();
    for name in ["", ".", "..", "a/b", "nul\0name"] {
        let error = tree.root.descend(OsStr::new(name)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{name:?}");
        assert!(tree
            .root
            .open_file(OsStr::new(name), ContainedOpenOptions::write_create(0o600))
            .is_err());
    }
    assert_eq!(tree.root.entries().unwrap().len(), 0);
}

#[test]
fn long_hostile_names_fail_without_mutating_the_tree() {
    let tree = tree();
    let long = "a".repeat(4096);
    assert!(tree
        .root
        .open_file(OsStr::new(&long), ContainedOpenOptions::write_create(0o600))
        .is_err());
    assert_eq!(tree.root.entries().unwrap().len(), 0);
}

#[test]
fn symlink_loops_and_directories_are_never_opened_as_files() {
    let tree = tree();
    symlink("loop", tree.root_path.join("loop")).unwrap();
    std::fs::create_dir(tree.root_path.join("directory")).unwrap();

    let loop_error = tree
        .root
        .open_file(OsStr::new("loop"), ContainedOpenOptions::read_only())
        .unwrap_err();
    assert!(is_symlink_refusal(&loop_error));
    assert!(tree
        .root
        .open_file(OsStr::new("directory"), ContainedOpenOptions::read_only())
        .is_err());
}

#[test]
fn listings_preserve_non_utf8_names_without_following_dangling_links() {
    let tree = tree();
    let non_utf8 = OsStr::from_bytes(b"name-\xff");
    std::fs::write(tree.root_path.join(non_utf8), b"data").unwrap();
    symlink("absent", tree.root_path.join("dangling")).unwrap();

    let kinds: std::collections::HashMap<_, _> = tree
        .root
        .entries()
        .unwrap()
        .into_iter()
        .map(|entry| (entry.name, entry.kind))
        .collect();
    assert_eq!(kinds[non_utf8], EntryKind::File);
    assert_eq!(kinds[OsStr::new("dangling")], EntryKind::Other);
}

#[test]
fn not_directory_errno_is_classified_without_exposing_nix() {
    let tree = tree();
    std::fs::write(tree.root_path.join("file"), b"data").unwrap();
    let error = tree.root.descend(OsStr::new("file")).unwrap_err();
    assert!(is_not_directory(&error));
}
