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
