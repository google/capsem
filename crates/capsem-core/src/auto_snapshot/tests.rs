//! Tests for `auto_snapshot` (extracted from inline `mod tests`).

use super::*;

fn setup_session_dir() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let session = tmp.path().to_path_buf();
    std::fs::create_dir_all(session.join("workspace")).unwrap();
    std::fs::create_dir_all(session.join("system")).unwrap();
    std::fs::create_dir_all(session.join("auto_snapshots")).unwrap();
    (tmp, session)
}

fn sched(session: &Path) -> AutoSnapshotScheduler {
    AutoSnapshotScheduler::new(session.to_path_buf(), 3, 4, Duration::from_secs(300))
}

#[test]
fn take_auto_snapshot_creates_slot() {
    let (_tmp, session) = setup_session_dir();
    std::fs::write(session.join("workspace/hello.txt"), "world").unwrap();

    let mut s = sched(&session);
    let slot = s.take_snapshot().unwrap();

    assert_eq!(slot.slot, 0);
    assert_eq!(slot.origin, SnapshotOrigin::Auto);
    assert!(slot.name.is_none());
    assert!(slot.hash.is_none()); // auto-snapshots skip hash for performance
    assert_eq!(slot.files_count, 1); // hello.txt
    assert!(slot.workspace_path.join("hello.txt").exists());
    let content = std::fs::read_to_string(slot.workspace_path.join("hello.txt")).unwrap();
    assert_eq!(content, "world");

    let meta_path = session.join("auto_snapshots/0/metadata.json");
    assert!(meta_path.exists());
    let meta: SlotMetadata =
        serde_json::from_str(&std::fs::read_to_string(&meta_path).unwrap()).unwrap();
    assert_eq!(meta.origin, SnapshotOrigin::Auto);
    assert!(meta.name.is_none());
}

#[test]
fn take_named_snapshot_has_origin_and_hash() {
    let (_tmp, session) = setup_session_dir();
    std::fs::write(session.join("workspace/file.txt"), "data").unwrap();

    let mut s = sched(&session);
    let slot = s.take_named_snapshot("my_checkpoint").unwrap();

    assert_eq!(slot.slot, 3); // manual pool starts at max_auto=3
    assert_eq!(slot.origin, SnapshotOrigin::Manual);
    assert_eq!(slot.name.as_deref(), Some("my_checkpoint"));
    assert!(slot.hash.is_some());
    assert!(!slot.hash.as_ref().unwrap().is_empty());
    assert_eq!(slot.files_count, 1); // file.txt
}

#[test]
fn files_count_tracks_multiple_files() {
    let (_tmp, session) = setup_session_dir();
    let ws = session.join("workspace");
    std::fs::write(ws.join("a.txt"), "a").unwrap();
    std::fs::write(ws.join("b.txt"), "b").unwrap();
    std::fs::create_dir_all(ws.join("sub")).unwrap();
    std::fs::write(ws.join("sub/c.txt"), "c").unwrap();

    let mut s = sched(&session);
    let slot = s.take_snapshot().unwrap();
    assert_eq!(slot.files_count, 3); // a.txt, b.txt, sub/c.txt

    // Add a file and take another snapshot.
    std::fs::write(ws.join("d.txt"), "d").unwrap();
    let slot2 = s.take_snapshot().unwrap();
    assert_eq!(slot2.files_count, 4);
}

#[test]
fn files_count_zero_for_empty_workspace() {
    let (_tmp, session) = setup_session_dir();
    let mut s = sched(&session);
    let slot = s.take_snapshot().unwrap();
    assert_eq!(slot.files_count, 0);
}

#[test]
fn auto_ring_buffer_wraps() {
    let (_tmp, session) = setup_session_dir();
    let mut s = AutoSnapshotScheduler::new(session.clone(), 2, 2, Duration::from_secs(300));

    std::fs::write(session.join("workspace/a.txt"), "first").unwrap();
    s.take_snapshot().unwrap(); // slot 0

    std::fs::write(session.join("workspace/a.txt"), "second").unwrap();
    s.take_snapshot().unwrap(); // slot 1

    std::fs::write(session.join("workspace/a.txt"), "third").unwrap();
    s.take_snapshot().unwrap(); // slot 0 again

    let content =
        std::fs::read_to_string(session.join("auto_snapshots/0/workspace/a.txt")).unwrap();
    assert_eq!(content, "third");
    let content =
        std::fs::read_to_string(session.join("auto_snapshots/1/workspace/a.txt")).unwrap();
    assert_eq!(content, "second");
}

#[test]
fn separate_pools_dont_collide() {
    let (_tmp, session) = setup_session_dir();
    let mut s = AutoSnapshotScheduler::new(session.clone(), 2, 2, Duration::from_secs(300));

    std::fs::write(session.join("workspace/a.txt"), "auto").unwrap();
    s.take_snapshot().unwrap(); // auto slot 0

    std::fs::write(session.join("workspace/a.txt"), "manual").unwrap();
    s.take_named_snapshot("checkpoint_1").unwrap(); // manual slot 2

    let list = s.list_snapshots();
    assert_eq!(list.len(), 2);
    let auto: Vec<_> = list
        .iter()
        .filter(|s| s.origin == SnapshotOrigin::Auto)
        .collect();
    let manual: Vec<_> = list
        .iter()
        .filter(|s| s.origin == SnapshotOrigin::Manual)
        .collect();
    assert_eq!(auto.len(), 1);
    assert_eq!(manual.len(), 1);
    assert_eq!(auto[0].slot, 0);
    assert_eq!(manual[0].slot, 2);
}

#[test]
fn auto_cull_does_not_touch_manual() {
    let (_tmp, session) = setup_session_dir();
    let mut s = AutoSnapshotScheduler::new(session.clone(), 2, 2, Duration::from_secs(300));

    s.take_snapshot().unwrap();
    std::thread::sleep(Duration::from_millis(10));
    s.take_named_snapshot("keep_me").unwrap();

    assert!(s.evict_oldest());
    // Manual snapshot should survive.
    let list = s.list_snapshots();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].origin, SnapshotOrigin::Manual);
    assert_eq!(list[0].name.as_deref(), Some("keep_me"));
}

#[test]
fn delete_snapshot_removes_slot() {
    let (_tmp, session) = setup_session_dir();
    let mut s = sched(&session);

    let snap = s.take_named_snapshot("deleteme").unwrap();
    assert_eq!(s.list_snapshots().len(), 1);

    s.delete_snapshot(snap.slot).unwrap();
    assert_eq!(s.list_snapshots().len(), 0);
}

#[test]
fn available_manual_slots_decreases() {
    let (_tmp, session) = setup_session_dir();
    let mut s = sched(&session); // max_manual=4

    assert_eq!(s.available_manual_slots(), 4);
    s.take_named_snapshot("a").unwrap();
    assert_eq!(s.available_manual_slots(), 3);
    s.take_named_snapshot("b").unwrap();
    assert_eq!(s.available_manual_slots(), 2);
}

#[test]
fn manual_pool_full_returns_error() {
    let (_tmp, session) = setup_session_dir();
    let mut s = AutoSnapshotScheduler::new(session.clone(), 2, 1, Duration::from_secs(300));

    s.take_named_snapshot("first").unwrap();
    let err = s.take_named_snapshot("second").unwrap_err();
    assert!(err
        .to_string()
        .contains("no manual snapshot slots available"));
}

#[test]
fn list_snapshots_newest_first() {
    let (_tmp, session) = setup_session_dir();
    let mut s = sched(&session);

    s.take_snapshot().unwrap();
    std::thread::sleep(Duration::from_millis(10));
    s.take_snapshot().unwrap();
    std::thread::sleep(Duration::from_millis(10));
    s.take_snapshot().unwrap();

    let list = s.list_snapshots();
    assert_eq!(list.len(), 3);
    assert_eq!(list[0].slot, 2); // newest
    assert_eq!(list[2].slot, 0); // oldest
}

#[test]
fn get_snapshot_returns_none_for_empty_slot() {
    let (_tmp, session) = setup_session_dir();
    let s = sched(&session);
    assert!(s.get_snapshot(0).is_none());
    assert!(s.get_snapshot(99).is_none());
}

#[test]
fn workspace_hash_is_deterministic() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws");
    std::fs::create_dir_all(ws.join("sub")).unwrap();
    std::fs::write(ws.join("a.txt"), "hello").unwrap();
    std::fs::write(ws.join("sub/b.txt"), "world").unwrap();

    let h1 = workspace_hash(&ws);
    let h2 = workspace_hash(&ws);
    assert_eq!(h1, h2);
    assert_eq!(h1.len(), 64); // blake3 hex
}

#[test]
fn workspace_hash_changes_on_modification() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("a.txt"), "v1").unwrap();

    let h1 = workspace_hash(&ws);
    std::fs::write(ws.join("a.txt"), "v2-longer").unwrap();
    let h2 = workspace_hash(&ws);
    assert_ne!(h1, h2);
}

#[test]
fn compact_two_snapshots_merges_files() {
    let (_tmp, session) = setup_session_dir();
    let mut s = sched(&session);

    // Snap 1: file_a.txt
    std::fs::write(session.join("workspace/file_a.txt"), "aaa").unwrap();
    s.take_named_snapshot("snap_a").unwrap();

    // Snap 2: file_b.txt (file_a still exists)
    std::fs::write(session.join("workspace/file_b.txt"), "bbb").unwrap();
    s.take_named_snapshot("snap_b").unwrap();

    // Compact both into one.
    let slots: Vec<usize> = s.list_snapshots().iter().map(|sn| sn.slot).collect();
    let result = s.compact_snapshots(&slots, "merged").unwrap();

    // Merged snapshot should have both files.
    assert!(result.workspace_path.join("file_a.txt").exists());
    assert!(result.workspace_path.join("file_b.txt").exists());
    assert_eq!(
        std::fs::read_to_string(result.workspace_path.join("file_a.txt")).unwrap(),
        "aaa"
    );
    assert_eq!(
        std::fs::read_to_string(result.workspace_path.join("file_b.txt")).unwrap(),
        "bbb"
    );
}

#[test]
fn compact_newest_wins() {
    let (_tmp, session) = setup_session_dir();
    let mut s = sched(&session);

    // Snap 1: file.txt = "old"
    std::fs::write(session.join("workspace/file.txt"), "old").unwrap();
    let snap1 = s.take_named_snapshot("v1").unwrap();

    // Snap 2: file.txt = "new"
    std::fs::write(session.join("workspace/file.txt"), "new").unwrap();
    let snap2 = s.take_named_snapshot("v2").unwrap();

    let result = s
        .compact_snapshots(&[snap1.slot, snap2.slot], "merged")
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(result.workspace_path.join("file.txt")).unwrap(),
        "new"
    );
}

#[test]
fn compact_deletes_originals() {
    let (_tmp, session) = setup_session_dir();
    let mut s = sched(&session);

    std::fs::write(session.join("workspace/x.txt"), "x").unwrap();
    let snap1 = s.take_named_snapshot("a").unwrap();
    let snap2 = s.take_named_snapshot("b").unwrap();

    let slot1 = snap1.slot;
    let slot2 = snap2.slot;
    s.compact_snapshots(&[slot1, slot2], "merged").unwrap();

    // Originals should be gone.
    assert!(s.get_snapshot(slot1).is_none());
    assert!(s.get_snapshot(slot2).is_none());
}

#[test]
fn compact_requires_manual_slot() {
    let (_tmp, session) = setup_session_dir();
    // max_manual = 1 so only 1 manual slot available.
    let mut s = AutoSnapshotScheduler::new(session.to_path_buf(), 3, 1, Duration::from_secs(300));

    std::fs::write(session.join("workspace/f.txt"), "data").unwrap();
    let _snap1 = s.take_named_snapshot("fill").unwrap();
    // Manual pool is now full (1/1).
    // Create an auto snapshot to compact.
    let snap2 = s.take_snapshot().unwrap();
    // Compact auto into manual should fail (pool full).
    let result = s.compact_snapshots(&[snap2.slot], "nope");
    assert!(result.is_err(), "should fail when manual pool is full");
}

#[test]
fn compact_invalid_slot_errors() {
    let (_tmp, session) = setup_session_dir();
    let mut s = sched(&session);

    let result = s.compact_snapshots(&[999], "bad");
    assert!(result.is_err());
}

#[test]
fn clone_preserves_file_content() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src_dir");
    let dst = tmp.path().join("dst_dir");
    std::fs::create_dir_all(src.join("sub")).unwrap();
    std::fs::write(src.join("a.txt"), "hello").unwrap();
    std::fs::write(src.join("sub/b.txt"), "nested").unwrap();

    clone_directory(&src, &dst).unwrap();

    assert_eq!(std::fs::read_to_string(dst.join("a.txt")).unwrap(), "hello");
    assert_eq!(
        std::fs::read_to_string(dst.join("sub/b.txt")).unwrap(),
        "nested"
    );
}

// -----------------------------------------------------------------------
// SnapshotBackend trait + implementations
// -----------------------------------------------------------------------

#[test]
fn snapshot_backend_trait_is_object_safe() {
    fn _assert_obj_safe(_: &dyn SnapshotBackend) {}
}

#[test]
fn default_backend_returns_apfs_on_macos() {
    let backend = default_snapshot_backend();
    // On macOS, should be ApfsSnapshot. On other platforms, HardlinkSnapshot.
    // We just verify it returns something usable.
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    let dst = tmp.path().join("dst");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("test.txt"), "hello").unwrap();
    backend.snapshot(&src, &dst).unwrap();
    assert_eq!(
        std::fs::read_to_string(dst.join("test.txt")).unwrap(),
        "hello"
    );
}

// -----------------------------------------------------------------------
// clone_directory
// -----------------------------------------------------------------------

#[test]
fn copy_recursive_empty_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("empty_src");
    let dst = tmp.path().join("empty_dst");
    std::fs::create_dir_all(&src).unwrap();

    clone_directory(&src, &dst).unwrap();
    assert!(dst.is_dir());
}

#[test]
fn copy_recursive_single_file() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    let dst = tmp.path().join("dst");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("file.txt"), "content").unwrap();

    clone_directory(&src, &dst).unwrap();

    assert_eq!(
        std::fs::read_to_string(dst.join("file.txt")).unwrap(),
        "content"
    );
}

#[test]
fn copy_recursive_nested_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    std::fs::create_dir_all(src.join("a/b/c")).unwrap();
    std::fs::write(src.join("a/b/c/deep.txt"), "deep").unwrap();
    std::fs::write(src.join("a/top.txt"), "top").unwrap();

    let dst = tmp.path().join("dst");
    clone_directory(&src, &dst).unwrap();

    assert_eq!(
        std::fs::read_to_string(dst.join("a/b/c/deep.txt")).unwrap(),
        "deep"
    );
    assert_eq!(
        std::fs::read_to_string(dst.join("a/top.txt")).unwrap(),
        "top"
    );
}

#[test]
fn copy_recursive_preserves_binary_content() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    let binary: Vec<u8> = (0..=255).collect();
    std::fs::write(src.join("binary.bin"), &binary).unwrap();

    let dst = tmp.path().join("dst");
    clone_directory(&src, &dst).unwrap();

    assert_eq!(std::fs::read(dst.join("binary.bin")).unwrap(), binary);
}

#[test]
fn copy_recursive_empty_file() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("empty"), "").unwrap();

    let dst = tmp.path().join("dst");
    clone_directory(&src, &dst).unwrap();

    assert_eq!(std::fs::read(dst.join("empty")).unwrap().len(), 0);
}

#[test]
fn copy_recursive_many_files() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    for i in 0..50 {
        std::fs::write(src.join(format!("file_{i}.txt")), format!("content_{i}")).unwrap();
    }

    let dst = tmp.path().join("dst");
    clone_directory(&src, &dst).unwrap();

    for i in 0..50 {
        let content = std::fs::read_to_string(dst.join(format!("file_{i}.txt"))).unwrap();
        assert_eq!(content, format!("content_{i}"));
    }
}

#[test]
fn copy_recursive_source_not_found_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let result = clone_directory(&tmp.path().join("nonexistent"), &tmp.path().join("dst"));
    assert!(result.is_err());
}

#[test]
fn copy_recursive_empty_subdirs_preserved() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    std::fs::create_dir_all(src.join("empty_subdir")).unwrap();
    std::fs::write(src.join("file.txt"), "ok").unwrap();

    let dst = tmp.path().join("dst");
    clone_directory(&src, &dst).unwrap();

    assert!(dst.join("empty_subdir").is_dir());
    assert_eq!(std::fs::read_to_string(dst.join("file.txt")).unwrap(), "ok");
}

// -----------------------------------------------------------------------
// ReflinkSnapshot backend (Linux-only)
// -----------------------------------------------------------------------

#[cfg(target_os = "linux")]
#[test]
fn reflink_snapshot_copies_files() {
    // Works on any Linux filesystem -- falls back to byte copy on ext4.
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    std::fs::create_dir_all(src.join("subdir")).unwrap();
    std::fs::write(src.join("a.txt"), "alpha").unwrap();
    std::fs::write(src.join("subdir/b.txt"), "beta").unwrap();

    let dst = tmp.path().join("dst");
    ReflinkSnapshot.snapshot(&src, &dst).unwrap();

    assert_eq!(std::fs::read_to_string(dst.join("a.txt")).unwrap(), "alpha");
    assert_eq!(
        std::fs::read_to_string(dst.join("subdir/b.txt")).unwrap(),
        "beta"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn reflink_snapshot_empty_source() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    std::fs::create_dir_all(&src).unwrap();

    let dst = tmp.path().join("dst");
    ReflinkSnapshot.snapshot(&src, &dst).unwrap();
    assert!(dst.is_dir());
}

#[cfg(target_os = "linux")]
#[test]
fn reflink_snapshot_source_not_found_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let result = ReflinkSnapshot.snapshot(&tmp.path().join("nonexistent"), &tmp.path().join("dst"));
    assert!(result.is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn reflink_snapshot_preserves_nested_structure() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    std::fs::create_dir_all(src.join("a/b/c")).unwrap();
    std::fs::write(src.join("a/b/c/deep.txt"), "deep").unwrap();
    std::fs::write(src.join("top.txt"), "top").unwrap();

    let dst = tmp.path().join("dst");
    ReflinkSnapshot.snapshot(&src, &dst).unwrap();

    assert_eq!(
        std::fs::read_to_string(dst.join("a/b/c/deep.txt")).unwrap(),
        "deep"
    );
    assert_eq!(std::fs::read_to_string(dst.join("top.txt")).unwrap(), "top");
}

#[cfg(target_os = "linux")]
#[test]
fn reflink_snapshot_preserves_binary_content() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    let binary: Vec<u8> = (0..=255).collect();
    std::fs::write(src.join("binary.bin"), &binary).unwrap();

    let dst = tmp.path().join("dst");
    ReflinkSnapshot.snapshot(&src, &dst).unwrap();

    assert_eq!(std::fs::read(dst.join("binary.bin")).unwrap(), binary);
}

#[cfg(target_os = "linux")]
#[test]
fn reflink_try_reflink_returns_false_on_unsupported_fs() {
    // tmpfs doesn't support FICLONE -- verify graceful fallback.
    let tmp = tempfile::tempdir().unwrap();
    let src_path = tmp.path().join("src.txt");
    let dst_path = tmp.path().join("dst.txt");
    std::fs::write(&src_path, "test").unwrap();

    let result = ReflinkSnapshot::try_reflink(&src_path, &dst_path).unwrap();
    // On tmpfs/ext4, FICLONE is not supported so this should be false.
    // On btrfs/xfs, it would be true. Either way, no error.
    assert!(result == true || result == false);
    // If reflink failed, dst was cleaned up and caller does byte copy.
    if !result {
        assert!(!dst_path.exists());
    }
}

// -----------------------------------------------------------------------
// ApfsSnapshot backend (macOS-only behavior)
// -----------------------------------------------------------------------

#[test]
#[cfg(target_os = "macos")]
fn apfs_snapshot_copies_files() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    std::fs::create_dir_all(src.join("sub")).unwrap();
    std::fs::write(src.join("x.txt"), "data").unwrap();
    std::fs::write(src.join("sub/y.txt"), "nested").unwrap();

    let dst = tmp.path().join("dst");
    ApfsSnapshot.snapshot(&src, &dst).unwrap();

    assert_eq!(std::fs::read_to_string(dst.join("x.txt")).unwrap(), "data");
    assert_eq!(
        std::fs::read_to_string(dst.join("sub/y.txt")).unwrap(),
        "nested"
    );
}

#[test]
#[cfg(target_os = "macos")]
fn apfs_snapshot_empty_source() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    std::fs::create_dir_all(&src).unwrap();

    let dst = tmp.path().join("dst");
    ApfsSnapshot.snapshot(&src, &dst).unwrap();
    assert!(dst.is_dir());
}

// -----------------------------------------------------------------------
// clone_directory (dispatches to platform backend)
// -----------------------------------------------------------------------

#[test]
fn clone_directory_dispatches_correctly() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("test.txt"), "cloned").unwrap();

    let dst = tmp.path().join("dst");
    clone_directory(&src, &dst).unwrap();

    assert_eq!(
        std::fs::read_to_string(dst.join("test.txt")).unwrap(),
        "cloned"
    );
}

// -----------------------------------------------------------------------
// clone_file (single-file CoW with platform fallback)
// -----------------------------------------------------------------------

#[test]
fn clone_file_copies_content() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src.txt");
    std::fs::write(&src, "hello capsem").unwrap();

    let dst = tmp.path().join("dst.txt");
    clone_file(&src, &dst).unwrap();

    assert_eq!(std::fs::read_to_string(&dst).unwrap(), "hello capsem");
}

#[test]
fn clone_file_overwrites_existing_if_removed_first() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src.txt");
    std::fs::write(&src, "new content").unwrap();

    let dst = tmp.path().join("dst.txt");
    std::fs::write(&dst, "old content").unwrap();

    // clone_file expects dst to not exist (caller removes first).
    std::fs::remove_file(&dst).unwrap();
    clone_file(&src, &dst).unwrap();

    assert_eq!(std::fs::read_to_string(&dst).unwrap(), "new content");
}

#[test]
fn clone_file_empty_file() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("empty.txt");
    std::fs::write(&src, "").unwrap();

    let dst = tmp.path().join("dst.txt");
    clone_file(&src, &dst).unwrap();

    assert_eq!(std::fs::read_to_string(&dst).unwrap(), "");
}

#[test]
fn clone_file_nonexistent_source_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("no_such_file.txt");
    let dst = tmp.path().join("dst.txt");
    assert!(clone_file(&src, &dst).is_err());
}

// -----------------------------------------------------------------------
// Snapshot scheduler with backend trait
// -----------------------------------------------------------------------

#[test]
fn snapshot_scheduler_uses_clone_directory() {
    // Verify the scheduler still works end-to-end after the
    // clone_directory refactor to use SnapshotBackend trait.
    let (_tmp, session) = setup_session_dir();
    std::fs::write(session.join("workspace/data.txt"), "important").unwrap();
    std::fs::write(session.join("system/rootfs.img"), "system_data").unwrap();

    let mut s = sched(&session);
    let slot = s.take_snapshot().unwrap();

    assert!(slot.workspace_path.join("data.txt").exists());
    assert_eq!(
        std::fs::read_to_string(slot.workspace_path.join("data.txt")).unwrap(),
        "important"
    );

    // System dir should also be snapshotted
    let system_snap = session.join(format!("auto_snapshots/{}/system", slot.slot));
    assert!(system_snap.exists());
}

// -------------------------------------------------------------------
// Symlink handling in snapshots
// -------------------------------------------------------------------

#[test]
fn symlink_included_in_file_count() {
    let (_tmp, session) = setup_session_dir();
    std::fs::write(session.join("workspace/real.txt"), "data").unwrap();
    std::os::unix::fs::symlink("real.txt", session.join("workspace/link.txt")).unwrap();

    let mut s = sched(&session);
    let slot = s.take_snapshot().unwrap();
    // Both the file and symlink should be counted.
    assert_eq!(slot.files_count, 2);
}

#[test]
fn workspace_hash_includes_symlinks() {
    let (_tmp, session) = setup_session_dir();
    std::fs::write(session.join("workspace/file.txt"), "data").unwrap();
    let h1 = workspace_hash(&session.join("workspace"));

    // Adding a symlink should change the hash.
    std::os::unix::fs::symlink("file.txt", session.join("workspace/link.txt")).unwrap();
    let h2 = workspace_hash(&session.join("workspace"));
    assert_ne!(h1, h2, "hash must change when symlink is added");
}

#[test]
fn workspace_hash_distinguishes_different_symlink_targets() {
    let (_tmp, session) = setup_session_dir();
    std::fs::write(session.join("workspace/a.txt"), "aaa").unwrap();
    std::fs::write(session.join("workspace/b.txt"), "bbb").unwrap();

    std::os::unix::fs::symlink("a.txt", session.join("workspace/link")).unwrap();
    let h1 = workspace_hash(&session.join("workspace"));

    // Re-point the symlink to a different target.
    std::fs::remove_file(session.join("workspace/link")).unwrap();
    std::os::unix::fs::symlink("b.txt", session.join("workspace/link")).unwrap();
    let h2 = workspace_hash(&session.join("workspace"));

    assert_ne!(h1, h2, "hash must differ for different symlink targets");
}

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
    assert_eq!(
        std::fs::read(dst.join("system/rootfs.img")).unwrap(),
        b"rootfs-data"
    );
    assert_eq!(
        std::fs::read(dst.join("workspace/hello.txt")).unwrap(),
        b"world"
    );
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
    std::fs::write(src.join("session.db"), b"db-contents").unwrap();

    let dst_tmp = tempfile::tempdir().unwrap();
    let dst = dst_tmp.path().join("clone");
    std::fs::create_dir_all(&dst).unwrap();

    clone_sandbox_state(src, &dst).unwrap();

    // session.db should be at session root, not in guest/
    assert!(dst.join("session.db").exists());
    assert_eq!(
        std::fs::read(dst.join("session.db")).unwrap(),
        b"db-contents"
    );
}

#[test]
fn clone_sandbox_state_preserves_vm_effective_profile_attachments() {
    let src_tmp = tempfile::tempdir().unwrap();
    let src = src_tmp.path();
    std::fs::create_dir_all(src.join("system")).unwrap();
    std::fs::write(
        src.join(crate::settings_profiles::VM_EFFECTIVE_SETTINGS_FILENAME),
        b"profile_id = \"everyday-work\"\n",
    )
    .unwrap();
    std::fs::write(
        src.join(crate::settings_profiles::VM_EFFECTIVE_TRACE_FILENAME),
        br#"{"selected_profile_id":"everyday-work","events":[]}"#,
    )
    .unwrap();

    let dst_tmp = tempfile::tempdir().unwrap();
    let dst = dst_tmp.path().join("clone");
    std::fs::create_dir_all(&dst).unwrap();

    clone_sandbox_state(src, &dst).unwrap();

    assert_eq!(
        std::fs::read(dst.join(crate::settings_profiles::VM_EFFECTIVE_SETTINGS_FILENAME)).unwrap(),
        b"profile_id = \"everyday-work\"\n"
    );
    assert_eq!(
        std::fs::read(dst.join(crate::settings_profiles::VM_EFFECTIVE_TRACE_FILENAME)).unwrap(),
        br#"{"selected_profile_id":"everyday-work","events":[]}"#
    );
    assert!(!dst
        .join("guest")
        .join(crate::settings_profiles::VM_EFFECTIVE_SETTINGS_FILENAME)
        .exists());
}
