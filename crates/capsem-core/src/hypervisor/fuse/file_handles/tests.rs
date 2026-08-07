use super::*;
use std::path::PathBuf;

fn temp_share(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("capsem-fuse-test").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn alloc_and_remove() {
    let dir = temp_share("fh-alloc");
    std::fs::write(dir.join("f.txt"), b"x").unwrap();
    let file = std::fs::File::open(dir.join("f.txt")).unwrap();
    let mut fht = FileHandleTable::new();
    let fh = fht.alloc(OpenHandle::File(file)).unwrap();
    assert!(fht.get_file(fh).is_some());
    fht.remove(fh);
    assert!(fht.get_file(fh).is_none());
}

#[test]
fn sequential_ids() {
    let dir = temp_share("fh-seq");
    std::fs::write(dir.join("a"), b"").unwrap();
    std::fs::write(dir.join("b"), b"").unwrap();
    let mut fht = FileHandleTable::new();
    let fh1 = fht
        .alloc(OpenHandle::File(
            std::fs::File::open(dir.join("a")).unwrap(),
        ))
        .unwrap();
    let fh2 = fht
        .alloc(OpenHandle::File(
            std::fs::File::open(dir.join("b")).unwrap(),
        ))
        .unwrap();
    assert_eq!(fh2, fh1 + 1);
}

#[test]
fn alloc_respects_limit() {
    let mut fht = FileHandleTable::with_limit(2);
    assert!(fht.alloc(OpenHandle::Dir(vec![])).is_some());
    assert!(fht.alloc(OpenHandle::Dir(vec![])).is_some());
    assert!(fht.alloc(OpenHandle::Dir(vec![])).is_none());
}

#[test]
fn alloc_after_remove_under_limit() {
    let mut fht = FileHandleTable::with_limit(1);
    let fh = fht.alloc(OpenHandle::Dir(vec![])).unwrap();
    fht.remove(fh);
    assert!(fht.alloc(OpenHandle::Dir(vec![])).is_some());
}
