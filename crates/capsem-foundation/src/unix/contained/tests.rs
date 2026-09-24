use std::ffi::OsStr;
use std::io::{Read, Write};
use std::os::unix::fs::symlink;
use std::path::Path;

use nix::sys::stat::Mode;

use super::*;

#[test]
fn link_reads_are_relative_to_the_open_directory_after_an_ancestor_is_replaced() {
    let tree = tree();
    let original = tree.root_path.join("child");
    std::fs::create_dir(&original).unwrap();
    symlink("original-target", original.join("link")).unwrap();
    symlink("outside-target", tree.outside.join("link")).unwrap();
    let child = tree.root.descend(OsStr::new("child")).unwrap();
    std::fs::rename(&original, tree.root_path.join("saved")).unwrap();
    symlink(&tree.outside, &original).unwrap();
    assert_eq!(
        child.read_link(OsStr::new("link")).unwrap(),
        Some("original-target".into())
    );
    assert!(child.read_link(OsStr::new("../link")).is_err());
    assert_eq!(tree.root.read_link(OsStr::new("saved")).unwrap(), None);
}

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
fn private_file_creation_is_exclusive_owner_only_and_directory_syncable() {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().unwrap();
    let dir = ContainedDir::open_root(root.path()).unwrap();
    let file = dir.create_new_private_file(std::ffi::OsStr::new("generation")).unwrap();
    assert!(file.as_raw_fd() >= 0);
    assert_eq!(file.metadata().unwrap().permissions().mode() & 0o777, 0o600);
    assert!(dir.create_new_private_file(std::ffi::OsStr::new("generation")).is_err());
    dir.sync().unwrap();
}

#[test]
fn existing_private_append_refuses_permissions_and_extra_links() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().unwrap();
    let dir = ContainedDir::open_root(root.path()).unwrap();
    let path = root.path().join("generation");
    std::fs::write(&path, b"bytes").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(dir.open_existing_private_append(OsStr::new("generation")).is_ok());

    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
    assert_eq!(
        dir.open_existing_private_append(OsStr::new("generation"))
            .unwrap_err()
            .kind(),
        io::ErrorKind::PermissionDenied
    );
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::hard_link(&path, root.path().join("extra-link")).unwrap();
    assert_eq!(
        dir.open_existing_private_append(OsStr::new("generation"))
            .unwrap_err()
            .kind(),
        io::ErrorKind::PermissionDenied
    );
}

#[test]
fn private_directory_validation_uses_the_opened_descriptor() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().unwrap();
    let private = root.path().join("private");
    std::fs::create_dir(&private).unwrap();
    std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o700)).unwrap();
    let dir = ContainedDir::open_root(&private).unwrap();
    dir.validate_private().unwrap();
    std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o750)).unwrap();
    assert_eq!(
        dir.validate_private().unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
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
fn listings_preserve_names_without_following_dangling_links() {
    check_listing_name(OsStr::new("name-☃"));
}

// APFS rejects invalid UTF-8 names at creation; Linux filesystems admit them.
#[cfg(target_os = "linux")]
#[test]
fn listings_preserve_non_utf8_names_without_following_dangling_links() {
    use std::os::unix::ffi::OsStrExt;
    check_listing_name(OsStr::from_bytes(b"name-\xff"));
}

fn check_listing_name(name: &OsStr) {
    let tree = tree();
    std::fs::write(tree.root_path.join(name), b"data").unwrap();
    symlink("absent", tree.root_path.join("dangling")).unwrap();

    let kinds: std::collections::HashMap<_, _> = tree
        .root
        .entries()
        .unwrap()
        .into_iter()
        .map(|entry| (entry.name, entry.kind))
        .collect();
    assert_eq!(kinds[name], EntryKind::File);
    assert_eq!(kinds[OsStr::new("dangling")], EntryKind::Other);
}

#[test]
fn not_directory_errno_is_classified_without_exposing_nix() {
    let tree = tree();
    std::fs::write(tree.root_path.join("file"), b"data").unwrap();
    let error = tree.root.descend(OsStr::new("file")).unwrap_err();
    assert!(is_not_directory(&error));
}

/// A writer controls mtime; it cannot control ctime or the inode. So an entry's
/// identity must move on a same-size edit even when mtime is put back, or a
/// comparison that trusts identity would miss the edit.
#[test]
fn entry_identity_moves_on_an_edit_even_when_mtime_is_restored() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("file");
    std::fs::write(&path, "before").unwrap();
    let dir = ContainedDir::open_root(root.path()).unwrap();
    let identity = |dir: &ContainedDir| {
        dir.entries()
            .unwrap()
            .into_iter()
            .find(|entry| entry.name == "file")
            .unwrap()
            .identity
    };
    let before = identity(&dir);
    // Past the filesystem's timestamp granularity, so ctime can move.
    std::thread::sleep(std::time::Duration::from_millis(20));

    std::fs::write(&path, "after!").unwrap();
    let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.set_modified(std::time::UNIX_EPOCH + std::time::Duration::new(before.mtime.0 as u64, before.mtime.1 as u32))
        .unwrap();
    let after = identity(&dir);

    assert_eq!(after.size, before.size, "a same-size edit");
    assert_eq!(after.mtime, before.mtime, "with mtime restored");
    assert_ne!(after, before, "still changes the identity, through ctime");
}

#[test]
fn create_new_refuses_every_existing_entry_including_links() {
    let tree = tree();
    std::fs::write(tree.root_path.join("file"), b"keep").unwrap();
    symlink(tree.outside.join("target"), tree.root_path.join("dangling")).unwrap();
    for name in ["file", "dangling"] {
        let error = tree
            .root
            .open_file(OsStr::new(name), ContainedOpenOptions::write_create_new(0o600))
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists, "{name}: {error}");
    }
    assert!(!tree.outside.join("target").exists(), "a dangling link was not written through");
    let mut fresh = tree
        .root
        .open_file(OsStr::new("fresh"), ContainedOpenOptions::write_create_new(0o640))
        .unwrap();
    fresh.write_all(b"new").unwrap();
    assert_eq!(std::fs::read(tree.root_path.join("fresh")).unwrap(), b"new");
}

#[test]
fn symlink_creation_stores_the_target_verbatim_and_never_replaces() {
    let tree = tree();
    tree.root
        .symlink(OsStr::new("link"), OsStr::new("../../etc/passwd"))
        .unwrap();
    assert_eq!(
        std::fs::read_link(tree.root_path.join("link")).unwrap(),
        Path::new("../../etc/passwd")
    );
    let error = tree.root.symlink(OsStr::new("link"), OsStr::new("other")).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert!(tree.root.symlink(OsStr::new("../escape"), OsStr::new("x")).is_err());
}

#[test]
fn mode_reads_permission_bits_from_the_descriptor() {
    use std::os::unix::fs::PermissionsExt;
    let tree = tree();
    std::fs::create_dir(tree.root_path.join("dir")).unwrap();
    std::fs::set_permissions(tree.root_path.join("dir"), std::fs::Permissions::from_mode(0o750)).unwrap();
    assert_eq!(tree.root.descend(OsStr::new("dir")).unwrap().mode().unwrap(), 0o750);
}
