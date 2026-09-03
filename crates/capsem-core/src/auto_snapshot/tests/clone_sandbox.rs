//! Tests for `clone_sandbox_state`.

use super::*;

#[test]
fn clone_sandbox_state_basic() {
    let src_tmp = tempfile::tempdir().unwrap();
    let src = src_tmp.path();
    std::fs::create_dir_all(src.join("system")).unwrap();
    std::fs::create_dir_all(src.join("workspace")).unwrap();
    std::fs::write(src.join("system/rootfs.img"), b"rootfs-data").unwrap();
    std::fs::write(src.join("workspace/hello.txt"), b"world").unwrap();

    let dst_tmp = tempfile::tempdir().unwrap();
    let dst = dst_tmp.path().join("clone");
    std::fs::create_dir_all(&dst).unwrap();

    let size = clone_sandbox_state(src, &dst).unwrap();
    assert!(size > 0);

    // Verify guest/ layout
    assert!(dst.join("guest/system/rootfs.img").exists());
    assert!(dst.join("guest/workspace/hello.txt").exists());
    // Verify compat symlinks
    assert!(dst.join("system").is_symlink());
    assert!(dst.join("workspace").is_symlink());
    assert_eq!(std::fs::read(dst.join("system/rootfs.img")).unwrap(), b"rootfs-data");
    assert_eq!(std::fs::read(dst.join("workspace/hello.txt")).unwrap(), b"world");
}

#[test]
fn clone_sandbox_state_empty_session() {
    let src_tmp = tempfile::tempdir().unwrap();
    let src = src_tmp.path();
    // No system/ or workspace/ dirs

    let dst_tmp = tempfile::tempdir().unwrap();
    let dst = dst_tmp.path().join("clone");
    std::fs::create_dir_all(&dst).unwrap();

    // Should succeed even with no content to clone
    let size = clone_sandbox_state(src, &dst).unwrap();
    assert_eq!(size, 0);
}

#[test]
fn clone_sandbox_state_with_session_db() {
    let src_tmp = tempfile::tempdir().unwrap();
    let src = src_tmp.path();
    std::fs::create_dir_all(src.join("system")).unwrap();
    let src_db = src.join("session.db");
    let conn = rusqlite::Connection::open(&src_db).unwrap();
    conn.execute_batch(
        "CREATE TABLE ledger (id INTEGER PRIMARY KEY, payload TEXT NOT NULL);
         INSERT INTO ledger (payload) VALUES ('db-contents');",
    )
    .unwrap();
    drop(conn);

    let dst_tmp = tempfile::tempdir().unwrap();
    let dst = dst_tmp.path().join("clone");
    std::fs::create_dir_all(&dst).unwrap();

    clone_sandbox_state(src, &dst).unwrap();

    // session.db should be at session root, not in guest/
    assert!(dst.join("session.db").exists());
    assert!(!dst.join("guest/session.db").exists());
    let cloned = rusqlite::Connection::open(dst.join("session.db")).unwrap();
    let payload: String = cloned
        .query_row("SELECT payload FROM ledger WHERE id = 1", [], |row| row.get(0))
        .unwrap();
    assert_eq!(payload, "db-contents");
    let quick_check: String = cloned
        .pragma_query_value(None, "quick_check", |row| row.get(0))
        .unwrap();
    assert_eq!(quick_check, "ok");
}

#[test]
fn clone_sandbox_state_snapshots_wal_backed_session_db() {
    let src_tmp = tempfile::tempdir().unwrap();
    let src = src_tmp.path();
    std::fs::create_dir_all(src.join("system")).unwrap();
    let src_db = src.join("session.db");
    let conn = rusqlite::Connection::open(&src_db).unwrap();
    let journal_mode: String = conn
        .pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))
        .unwrap();
    assert_eq!(journal_mode.to_lowercase(), "wal");
    conn.execute_batch(
        "CREATE TABLE ledger (id INTEGER PRIMARY KEY, payload TEXT NOT NULL);
         INSERT INTO ledger (payload) VALUES ('committed-in-wal');",
    )
    .unwrap();
    assert!(
        src.join("session.db-wal").exists(),
        "test must prove WAL sidecar exists before clone"
    );

    let dst_tmp = tempfile::tempdir().unwrap();
    let dst = dst_tmp.path().join("clone");
    std::fs::create_dir_all(&dst).unwrap();

    clone_sandbox_state(src, &dst).unwrap();

    assert!(dst.join("session.db").exists());
    assert!(!dst.join("session.db-wal").exists());
    assert!(!dst.join("session.db-shm").exists());
    let cloned = rusqlite::Connection::open(dst.join("session.db")).unwrap();
    let payload: String = cloned
        .query_row("SELECT payload FROM ledger WHERE id = 1", [], |row| row.get(0))
        .unwrap();
    assert_eq!(payload, "committed-in-wal");
    let quick_check: String = cloned
        .pragma_query_value(None, "quick_check", |row| row.get(0))
        .unwrap();
    assert_eq!(quick_check, "ok");
}
