use super::*;

#[test]
fn saved_ports_reject_malformed_duplicate_zero_and_oversized_records() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ports");
    assert!(read(&path).unwrap().is_empty());
    for body in [
        "no-json",
        "[{}]",
        "[{\"host\":0,\"guest\":1}]",
        "[{\"host\":1,\"guest\":0}]",
        "[{\"host\":1,\"guest\":1},{\"host\":1,\"guest\":2}]",
    ] {
        std::fs::write(&path, body).unwrap();
        assert!(read(&path).is_err());
    }
    std::fs::write(&path, vec![b' '; 4097]).unwrap();
    assert!(read(&path).is_err());
    std::fs::remove_file(&path).unwrap();
    let target = dir.path().join("target");
    std::fs::write(&target, "[]").unwrap();
    std::os::unix::fs::symlink(&target, &path).unwrap();
    assert!(read(&path).is_err());
}
