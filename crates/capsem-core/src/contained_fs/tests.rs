use std::ffi::OsStr;
use std::io::{Read, Write};
use std::os::unix::ffi::OsStrExt;
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

// ---------------------------------------------------------------------------
// Attacker-shaped names and trees
// ---------------------------------------------------------------------------

#[test]
fn hostile_component_names_never_reach_a_syscall() {
    let t = tree();
    let long = "a".repeat(4096);
    let bad: [&str; 9] = ["", ".", "..", "a/b", "/etc", "a/../b", "\u{0}", "etc\u{0}passwd", &long];
    let before = t.root.entries().unwrap().len();
    for name in bad {
        let err = t.root.descend(os(name)).unwrap_err();
        let kind = err.kind();
        assert!(
            kind == io::ErrorKind::InvalidInput || kind == io::ErrorKind::NotFound || err.raw_os_error().is_some(),
            "{name:?}: {err}"
        );
        assert!(
            t.root.open_file(os(name), OFlag::O_RDONLY, Mode::empty()).is_err(),
            "{name:?}"
        );
        assert!(
            t.root
                .open_file(
                    os(name),
                    OFlag::O_WRONLY | OFlag::O_CREAT,
                    Mode::from_bits_truncate(0o644)
                )
                .is_err(),
            "{name:?} must not be creatable"
        );
    }
    assert_eq!(
        t.root.entries().unwrap().len(),
        before,
        "nothing may be created in the root"
    );
    assert_eq!(
        std::fs::read_dir(&t.outside).unwrap().count(),
        1,
        "nothing may be created outside"
    );
}

#[test]
fn a_symlink_at_any_depth_is_refused_even_when_it_points_inside_the_root() {
    let t = tree();
    std::fs::create_dir_all(t.root_path.join("real/deep")).unwrap();
    std::fs::write(t.root_path.join("real/deep/file"), "x").unwrap();
    // Relative link to a sibling inside the root: still a symlink, still refused.
    symlink("real", t.root_path.join("alias")).unwrap();
    symlink("../real/deep", t.root_path.join("real/hop")).unwrap();

    for rel in ["alias/deep", "real/hop", "alias/deep/file", "real/hop/file"] {
        let err = t.root.walk(Path::new(rel)).unwrap_err();
        assert!(is_symlink_refusal(&err), "{rel}: {err}");
    }
    let err = t.root.walk(Path::new("real/deep")).unwrap();
    assert!(err.open_file(os("file"), OFlag::O_RDONLY, Mode::empty()).is_ok());
}

#[test]
fn symlink_loops_are_refused_not_followed() {
    let t = tree();
    symlink("loop", t.root_path.join("loop")).unwrap();
    symlink("b", t.root_path.join("a")).unwrap();
    symlink("a", t.root_path.join("b")).unwrap();
    for name in ["loop", "a", "b"] {
        let err = t.root.descend(os(name)).unwrap_err();
        assert!(is_symlink_refusal(&err), "{name}: {err}");
        let err = t.root.open_file(os(name), OFlag::O_RDONLY, Mode::empty()).unwrap_err();
        assert!(is_symlink_refusal(&err), "{name}: {err}");
    }
}

#[test]
fn walk_creating_never_follows_a_symlink_placed_at_the_leaf() {
    let t = tree();
    std::fs::create_dir_all(t.root_path.join("parent")).unwrap();
    symlink(&t.outside, t.root_path.join("parent/leaf")).unwrap();
    let err = t
        .root
        .walk_creating(Path::new("parent/leaf"), Mode::from_bits_truncate(0o755))
        .unwrap_err();
    assert!(is_symlink_refusal(&err), "{err}");
    let err = t
        .root
        .walk_creating(Path::new("parent/leaf/child"), Mode::from_bits_truncate(0o755))
        .unwrap_err();
    assert!(is_symlink_refusal(&err), "{err}");
    assert!(!t.outside.join("child").exists());
}

#[test]
fn entries_survive_dangling_links_devices_and_unreadable_names() {
    let t = tree();
    symlink("/nonexistent/target", t.root_path.join("dangling")).unwrap();
    nix::unistd::mkfifo(&t.root_path.join("fifo"), Mode::from_bits_truncate(0o644)).unwrap();
    std::fs::write(t.root_path.join(OsStr::from_bytes(b"caf\xc3\xa9-\xff")), "x").unwrap();

    let entries = t.root.entries().unwrap();
    let kinds: std::collections::HashMap<_, _> = entries.iter().map(|e| (e.name.clone(), e.kind)).collect();
    assert_eq!(kinds[OsStr::new("dangling")], EntryKind::Other);
    assert_eq!(kinds[OsStr::new("fifo")], EntryKind::Other);
    assert_eq!(kinds[OsStr::from_bytes(b"caf\xc3\xa9-\xff")], EntryKind::File);
}

#[test]
fn open_file_refuses_a_directory_named_as_a_file() {
    let t = tree();
    std::fs::create_dir_all(t.root_path.join("dir")).unwrap();
    let err = t.root.open_file(os("dir"), OFlag::O_RDONLY, Mode::empty()).unwrap_err();
    assert!(
        err.kind() == io::ErrorKind::InvalidInput || err.raw_os_error() == Some(nix::errno::Errno::EISDIR as i32),
        "{err}"
    );
}
