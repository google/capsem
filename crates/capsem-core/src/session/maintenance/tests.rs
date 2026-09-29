use super::*;

#[test]
fn disk_usage_empty_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let usage = disk_usage_bytes(tmp.path());
    assert_eq!(usage, 0);
}

#[test]
fn disk_usage_with_files() {
    let tmp = tempfile::tempdir().unwrap();
    let f1 = tmp.path().join("file1.txt");
    std::fs::write(&f1, "hello").unwrap();
    let usage = disk_usage_bytes(tmp.path());
    assert!(usage >= 5);
}

#[test]
fn disk_usage_nested_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    let sub = tmp.path().join("sub");
    std::fs::create_dir(&sub).unwrap();
    std::fs::write(sub.join("nested.txt"), "data").unwrap();
    let usage = disk_usage_bytes(tmp.path());
    assert!(usage >= 4);
}

#[test]
fn disk_usage_nonexistent_dir() {
    let usage = disk_usage_bytes(Path::new("/nonexistent/path/to/sessions"));
    assert_eq!(usage, 0);
}

// A session directory holds the guest-writable workspace. The walk reached
// subdirectories by path after a no-follow stat, so a guest that swapped a
// directory for a symlink in between made the host walk the link's target on
// every `/info`. It now walks descriptors: a link is never entered.
#[test]
fn disk_usage_never_enters_a_symlinked_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("big"), vec![1u8; 1 << 20]).unwrap();
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    std::os::unix::fs::symlink(outside.path(), workspace.join("escape")).unwrap();
    std::os::unix::fs::symlink(outside.path(), tmp.path().join("escape")).unwrap();

    assert!(
        disk_usage_bytes(tmp.path()) < 1 << 20,
        "a symlinked directory was walked"
    );
}

#[test]
fn disk_usage_counts_allocated_blocks_not_logical_size() {
    let tmp = tempfile::tempdir().unwrap();
    let sparse = std::fs::File::create(tmp.path().join("rootfs.img")).unwrap();
    sparse.set_len(1 << 30).unwrap();
    assert!(
        disk_usage_bytes(tmp.path()) < 1 << 20,
        "a sparse file counted its logical size"
    );
}

#[test]
fn disk_usage_of_a_hostile_deep_tree_terminates() {
    let tmp = tempfile::tempdir().unwrap();
    let mut dir = tmp.path().to_path_buf();
    for _ in 0..400 {
        dir.push("d");
        std::fs::create_dir(&dir).unwrap();
    }
    std::fs::write(tmp.path().join("top"), vec![1u8; 8192]).unwrap();
    assert!(disk_usage_bytes(tmp.path()) >= 8192);
}
