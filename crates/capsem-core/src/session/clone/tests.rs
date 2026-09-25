use std::os::unix::fs::symlink;
use std::path::PathBuf;

use super::*;

struct Sessions {
    _tmp: tempfile::TempDir,
    src: PathBuf,
    dst: PathBuf,
    /// Host data outside both sessions that a fork must never carry.
    secret: PathBuf,
}

fn sessions() -> Sessions {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    let dst = tmp.path().join("dst");
    std::fs::create_dir_all(src.join("system")).unwrap();
    std::fs::create_dir_all(src.join("guest/workspace")).unwrap();
    std::fs::create_dir(&dst).unwrap();
    let secret = tmp.path().join("host-secret");
    std::fs::write(&secret, b"host secret").unwrap();
    Sessions {
        _tmp: tmp,
        src,
        dst,
        secret,
    }
}

#[test]
fn clones_system_workspace_and_compat_links() {
    let s = sessions();
    std::fs::write(s.src.join("system/rootfs.img"), b"rootfs-data").unwrap();
    std::fs::write(s.src.join("guest/workspace/hello.txt"), b"world").unwrap();

    let size = clone_sandbox_state(&s.src, &s.dst).unwrap();

    assert!(size > 0);
    let system = s.dst.join("system");
    assert!(
        system.is_dir() && !system.is_symlink(),
        "the overlay stays out of the share"
    );
    assert_eq!(std::fs::read(system.join("rootfs.img")).unwrap(), b"rootfs-data");
    assert!(!s.dst.join("guest/system").exists());
    assert_eq!(
        std::fs::read(s.dst.join("guest/workspace/hello.txt")).unwrap(),
        b"world"
    );
    assert!(s.dst.join("workspace").is_symlink());
    assert_eq!(std::fs::read(s.dst.join("workspace/hello.txt")).unwrap(), b"world");
}

#[test]
fn a_session_from_before_the_single_share_layout_still_clones() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    let dst = tmp.path().join("dst");
    std::fs::create_dir_all(src.join("system")).unwrap();
    std::fs::create_dir_all(src.join("workspace")).unwrap();
    std::fs::write(src.join("system/rootfs.img"), b"legacy").unwrap();
    std::fs::write(src.join("workspace/a"), b"a").unwrap();
    std::fs::create_dir(&dst).unwrap();

    clone_sandbox_state(&src, &dst).unwrap();

    assert_eq!(std::fs::read(dst.join("system/rootfs.img")).unwrap(), b"legacy");
    assert_eq!(std::fs::read(dst.join("workspace/a")).unwrap(), b"a");
}

#[test]
fn a_source_with_its_overlay_in_the_share_is_moved_out_before_cloning() {
    let s = sessions();
    std::fs::remove_dir(s.src.join("system")).unwrap();
    std::fs::create_dir(s.src.join("guest/system")).unwrap();
    std::fs::write(s.src.join("guest/system/rootfs.img"), b"in-share").unwrap();
    symlink("guest/system", s.src.join("system")).unwrap();

    clone_sandbox_state(&s.src, &s.dst).unwrap();

    assert_eq!(std::fs::read(s.src.join("system/rootfs.img")).unwrap(), b"in-share");
    assert!(!s.src.join("system").is_symlink());
    assert_eq!(std::fs::read(s.dst.join("system/rootfs.img")).unwrap(), b"in-share");
}

#[test]
fn a_session_without_a_share_clones_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    let dst = tmp.path().join("dst");
    std::fs::create_dir(&src).unwrap();
    std::fs::create_dir(&dst).unwrap();
    assert_eq!(clone_sandbox_state(&src, &dst).unwrap(), 0);
}

#[test]
fn workspace_symlinks_are_carried_as_links_not_as_host_data() {
    let s = sessions();
    symlink(&s.secret, s.src.join("guest/workspace/link")).unwrap();
    clone_sandbox_state(&s.src, &s.dst).unwrap();
    let cloned = s.dst.join("guest/workspace/link");
    assert!(std::fs::symlink_metadata(&cloned).unwrap().file_type().is_symlink());
    assert_eq!(std::fs::read_link(&cloned).unwrap(), s.secret);
}

#[test]
fn a_guest_replacing_workspace_with_a_symlink_fails_the_clone() {
    let s = sessions();
    std::fs::remove_dir(s.src.join("guest/workspace")).unwrap();
    symlink(s.secret.parent().unwrap(), s.src.join("guest/workspace")).unwrap();

    let error = clone_sandbox_state(&s.src, &s.dst).unwrap_err();

    assert!(format!("{error:#}").contains("workspace"), "{error:#}");
    assert!(!s.dst.join("guest/workspace/host-secret").exists());
}

#[test]
fn a_guest_replacing_the_overlay_image_with_a_symlink_fails_the_clone() {
    // In the old layout the image sat in the share; the fork would boot
    // whatever a planted link names as its disk.
    let s = sessions();
    std::fs::remove_dir(s.src.join("system")).unwrap();
    std::fs::create_dir(s.src.join("guest/system")).unwrap();
    symlink(&s.secret, s.src.join("guest/system/rootfs.img")).unwrap();
    symlink("guest/system", s.src.join("system")).unwrap();

    let error = clone_sandbox_state(&s.src, &s.dst).unwrap_err();

    assert!(format!("{error:#}").contains("overlay"), "{error:#}");
    assert!(std::fs::symlink_metadata(s.dst.join("system/rootfs.img")).is_err());
    assert_eq!(std::fs::read(&s.secret).unwrap(), b"host secret");
}

#[test]
fn a_linked_overlay_in_the_destination_is_refused() {
    // Belt and braces for the post-flush swap: whatever the walker produced,
    // the fork's rootfs.img must be a regular file.
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("system")).unwrap();
    symlink("/etc/hosts", tmp.path().join("system/rootfs.img")).unwrap();
    let system = ContainedDir::open_root(&tmp.path().join("system")).unwrap();
    assert!(require_regular_overlay(&system).is_err());
    std::fs::remove_file(tmp.path().join("system/rootfs.img")).unwrap();
    std::fs::write(tmp.path().join("system/rootfs.img"), b"img").unwrap();
    assert!(require_regular_overlay(&system).is_ok());
}

#[test]
fn clone_file_refuses_an_existing_destination() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("template"), b"ext4").unwrap();
    std::fs::write(tmp.path().join("taken"), b"keep").unwrap();
    assert!(clone_file(&tmp.path().join("template"), &tmp.path().join("taken")).is_err());
    clone_file(&tmp.path().join("template"), &tmp.path().join("fresh")).unwrap();
    assert_eq!(std::fs::read(tmp.path().join("fresh")).unwrap(), b"ext4");
    assert_eq!(std::fs::read(tmp.path().join("taken")).unwrap(), b"keep");
}

#[test]
fn session_db_is_copied_coherently_outside_the_share() {
    let s = sessions();
    capsem_logger::DbWriter::open(&s.src.join("session.db"), 8)
        .unwrap()
        .shutdown_blocking();
    let conn = rusqlite::Connection::open(s.src.join("session.db")).unwrap();
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
        s.src.join("session.db-wal").exists(),
        "the row must still live in the WAL"
    );

    clone_sandbox_state(&s.src, &s.dst).unwrap();

    assert!(!s.dst.join("guest/session.db").exists());
    assert!(!s.dst.join("session.db-wal").exists());
    let cloned = rusqlite::Connection::open(s.dst.join("session.db")).unwrap();
    let payload: String = cloned
        .query_row("SELECT payload FROM ledger WHERE id = 1", [], |row| row.get(0))
        .unwrap();
    assert_eq!(payload, "committed-in-wal");
    drop(conn);
}
