use std::ffi::OsStr;
use std::io::{Read, Write};
use std::os::unix::fs::symlink;
use std::path::Path;

use nix::fcntl::OFlag;
use nix::sys::stat::Mode;

use super::*;

struct Tree {
    _dir: tempfile::TempDir,
    root: ContainedDir,
    root_path: std::path::PathBuf,
    outside: std::path::PathBuf,
}

/// A workspace root with an `outside` sibling the guest must never reach.
fn tree() -> Tree {
    let dir = tempfile::tempdir().unwrap();
    let root_path = dir.path().join("workspace");
    let outside = dir.path().join("outside");
    std::fs::create_dir_all(&root_path).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.txt"), "secret").unwrap();
    let root = ContainedDir::open_root(&root_path).unwrap();
    Tree {
        _dir: dir,
        root,
        root_path,
        outside,
    }
}

fn os(name: &str) -> &OsStr {
    OsStr::new(name)
}

#[test]
fn descend_refuses_a_symlinked_directory() {
    let t = tree();
    symlink(&t.outside, t.root_path.join("escape")).unwrap();

    let err = t.root.descend(os("escape")).unwrap_err();
    assert!(is_symlink_refusal(&err), "{err}");
}

#[test]
fn walk_refuses_a_symlink_in_the_middle_of_the_path() {
    let t = tree();
    std::fs::create_dir_all(t.outside.join("deep")).unwrap();
    symlink(&t.outside, t.root_path.join("escape")).unwrap();

    let err = t.root.walk(Path::new("escape/deep")).unwrap_err();
    assert!(is_symlink_refusal(&err), "{err}");
}

#[test]
fn walk_creating_does_not_create_through_a_symlinked_parent() {
    let t = tree();
    symlink(&t.outside, t.root_path.join("link")).unwrap();

    let err = t
        .root
        .walk_creating(Path::new("link/sub"), Mode::from_bits_truncate(0o755))
        .unwrap_err();
    assert!(is_symlink_refusal(&err), "{err}");
    assert!(!t.outside.join("sub").exists(), "mkdir must not land outside the root");
}

#[test]
fn walk_creating_makes_nested_directories_inside_the_root() {
    let t = tree();
    let leaf = t
        .root
        .walk_creating(Path::new("a/b/c"), Mode::from_bits_truncate(0o755))
        .unwrap();
    assert_eq!(leaf.path(), t.root_path.canonicalize().unwrap().join("a/b/c"));
    assert!(t.root_path.join("a/b/c").is_dir());
}

#[test]
fn open_file_refuses_a_dangling_symlink_even_when_creating() {
    let t = tree();
    let target = t.outside.join("authorized_keys");
    symlink(&target, t.root_path.join("notes.txt")).unwrap();

    let err = t
        .root
        .open_file(
            os("notes.txt"),
            OFlag::O_WRONLY | OFlag::O_CREAT | OFlag::O_TRUNC,
            Mode::from_bits_truncate(0o644),
        )
        .unwrap_err();
    assert!(is_symlink_refusal(&err), "{err}");
    assert!(!target.exists(), "the symlink target must not be created");
}

#[test]
fn open_file_refuses_a_symlink_to_an_existing_host_file() {
    let t = tree();
    symlink(t.outside.join("secret.txt"), t.root_path.join("leak.txt")).unwrap();

    let err = t
        .root
        .open_file(os("leak.txt"), OFlag::O_RDONLY, Mode::empty())
        .unwrap_err();
    assert!(is_symlink_refusal(&err), "{err}");
}

#[test]
fn open_file_refuses_a_fifo_without_blocking() {
    let t = tree();
    nix::unistd::mkfifo(&t.root_path.join("pipe"), Mode::from_bits_truncate(0o644)).unwrap();

    let err = t
        .root
        .open_file(os("pipe"), OFlag::O_RDONLY, Mode::empty())
        .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{err}");
}

#[test]
fn open_file_reads_and_writes_regular_files() {
    let t = tree();
    let mut file = t
        .root
        .open_file(
            os("hello.txt"),
            OFlag::O_WRONLY | OFlag::O_CREAT | OFlag::O_TRUNC,
            Mode::from_bits_truncate(0o644),
        )
        .unwrap();
    file.write_all(b"world").unwrap();
    drop(file);

    let mut file = t
        .root
        .open_file(os("hello.txt"), OFlag::O_RDONLY, Mode::empty())
        .unwrap();
    let mut text = String::new();
    file.read_to_string(&mut text).unwrap();
    assert_eq!(text, "world");
}

#[test]
fn entries_report_symlinks_as_other_and_never_follow_them() {
    let t = tree();
    std::fs::create_dir_all(t.root_path.join("src")).unwrap();
    std::fs::write(t.root_path.join("README.md"), "# hi").unwrap();
    symlink(&t.outside, t.root_path.join("peek")).unwrap();

    let mut entries = t.root.entries().unwrap();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    let kinds: Vec<(String, EntryKind)> = entries
        .iter()
        .map(|e| (e.name.to_string_lossy().into_owned(), e.kind))
        .collect();
    assert_eq!(
        kinds,
        vec![
            ("README.md".to_string(), EntryKind::File),
            ("peek".to_string(), EntryKind::Other),
            ("src".to_string(), EntryKind::Directory),
        ]
    );
    let readme = entries.iter().find(|e| e.name == "README.md").unwrap();
    assert_eq!(readme.size, 4);
    assert!(readme.mtime_secs > 0);
}

#[test]
fn entry_kind_distinguishes_absent_from_symlink() {
    let t = tree();
    symlink(&t.outside, t.root_path.join("peek")).unwrap();
    std::fs::write(t.root_path.join("file"), "x").unwrap();

    assert_eq!(t.root.entry_kind(os("missing")).unwrap(), None);
    assert_eq!(t.root.entry_kind(os("peek")).unwrap(), Some(EntryKind::Other));
    assert_eq!(t.root.entry_kind(os("file")).unwrap(), Some(EntryKind::File));
}

#[test]
fn components_with_separators_or_dots_are_rejected_before_any_syscall() {
    let t = tree();
    for bad in ["", ".", "..", "a/b"] {
        let err = t.root.descend(os(bad)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{bad:?}");
    }
    let err = t.root.walk(Path::new("/etc")).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    let err = t.root.walk(Path::new("a/../b")).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
}
