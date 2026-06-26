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

#[test]
fn vacuum_missing_db_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let result = vacuum_and_compress_session_db(tmp.path());
    assert!(result.is_err());
}

#[test]
fn vacuum_real_db() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute_batch("CREATE TABLE test (id INTEGER PRIMARY KEY)")
        .unwrap();
    conn.execute_batch("INSERT INTO test VALUES (1)").unwrap();
    drop(conn);

    let result = vacuum_and_compress_session_db(tmp.path());
    assert!(result.is_ok());
    let gz_size = result.unwrap();
    assert!(gz_size > 0);
    assert!(!db_path.exists());
    assert!(tmp.path().join("session.db.gz").exists());
}
