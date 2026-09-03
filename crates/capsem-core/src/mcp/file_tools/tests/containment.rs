//! Revert and history never resolve a guest path by name after checking it.
//!
//! The workspace is the VirtioFS share: the guest can swap any directory for
//! a symlink at any moment. A check on a path string followed by a
//! `create_dir_all` / `remove_file` / `write` on that same string re-walks
//! the parents and lands wherever the symlink now points. Every mutation
//! here happens relative to a directory handle obtained by an `O_NOFOLLOW`
//! walk from the workspace root.

use super::*;

fn revert(session: &Path, args: Value) -> JsonRpcResponse {
    handle_revert_file(
        &args,
        &setup_sched(session),
        &session.join("workspace"),
        Some(serde_json::json!(1)),
        None,
    )
}

fn setup_sched(session: &Path) -> AutoSnapshotScheduler {
    AutoSnapshotScheduler::new(session.to_path_buf(), 10, 12, Duration::from_secs(300))
}

fn history(session: &Path, sched: &AutoSnapshotScheduler, path: &str) -> JsonRpcResponse {
    handle_snapshots_history(
        &serde_json::json!({"path": path}),
        sched,
        &session.join("workspace"),
        Some(serde_json::json!(1)),
    )
}

#[cfg(unix)]
#[test]
fn history_refuses_a_symlinked_parent_in_the_workspace() {
    let (tmp, session, mut sched) = setup();
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.txt"), "0123456789").unwrap();
    std::fs::create_dir_all(session.join("workspace/sub")).unwrap();
    std::fs::write(session.join("workspace/sub/secret.txt"), "x").unwrap();
    sched.take_snapshot().unwrap();
    std::fs::remove_dir_all(session.join("workspace/sub")).unwrap();
    std::os::unix::fs::symlink(&outside, session.join("workspace/sub")).unwrap();

    let resp = history(&session, &sched, "sub/secret.txt");
    let err = resp.error.expect("history through a symlinked parent must be refused");
    assert!(err.message.contains("symlink"), "{}", err.message);
}

#[cfg(unix)]
#[test]
fn history_refuses_a_symlinked_leaf_in_the_workspace() {
    let (tmp, session, mut sched) = setup();
    let outside = tmp.path().join("outside.txt");
    std::fs::write(&outside, "0123456789").unwrap();
    std::fs::write(session.join("workspace/leaf.txt"), "x").unwrap();
    sched.take_snapshot().unwrap();
    std::fs::remove_file(session.join("workspace/leaf.txt")).unwrap();
    std::os::unix::fs::symlink(&outside, session.join("workspace/leaf.txt")).unwrap();

    let resp = history(&session, &sched, "leaf.txt");
    let err = resp
        .error
        .expect("history of a symlink must be refused, never sized through it");
    assert!(err.message.contains("symlink"), "{}", err.message);
}

#[cfg(unix)]
#[test]
fn history_refuses_a_symlink_captured_inside_a_snapshot() {
    let (tmp, session, mut sched) = setup();
    let outside = tmp.path().join("outside.txt");
    std::fs::write(&outside, "0123456789").unwrap();
    std::os::unix::fs::symlink(&outside, session.join("workspace/leaf.txt")).unwrap();
    sched.take_snapshot().unwrap();
    std::fs::remove_file(session.join("workspace/leaf.txt")).unwrap();
    std::fs::write(session.join("workspace/leaf.txt"), "regular").unwrap();

    let resp = history(&session, &sched, "leaf.txt");
    let err = resp.error.expect("a snapshot symlink must not be sized through");
    assert!(err.message.contains("symlink"), "{}", err.message);
}

#[test]
fn history_reports_sizes_for_regular_files() {
    let (_tmp, session, mut sched) = setup();
    std::fs::write(session.join("workspace/a.txt"), "one").unwrap();
    sched.take_snapshot().unwrap();
    std::fs::write(session.join("workspace/a.txt"), "three").unwrap();
    sched.take_snapshot().unwrap();
    std::fs::remove_file(session.join("workspace/a.txt")).unwrap();

    let resp = history(&session, &sched, "/root/a.txt");
    let text = resp.result.unwrap()["content"][0]["text"].as_str().unwrap().to_string();
    let value: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["current_size"], Value::Null);
    assert_eq!(value["versions"][0]["size"], 5);
    assert_eq!(value["versions"][0]["status"], "modified");
    assert_eq!(value["versions"][1]["size"], 3);
    assert_eq!(value["versions"][1]["status"], "new");
}

#[cfg(unix)]
#[test]
fn revert_refuses_a_symlinked_workspace_parent_and_touches_nothing_outside() {
    let (tmp, session, mut sched) = setup();
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("file.txt"), "outside data").unwrap();
    std::fs::create_dir_all(session.join("workspace/sub")).unwrap();
    std::fs::write(session.join("workspace/sub/file.txt"), "snapshot data").unwrap();
    sched.take_snapshot().unwrap();
    std::fs::remove_dir_all(session.join("workspace/sub")).unwrap();
    std::os::unix::fs::symlink(&outside, session.join("workspace/sub")).unwrap();

    let resp = revert(
        &session,
        serde_json::json!({"path": "sub/file.txt", "checkpoint": "cp-0"}),
    );
    let err = resp.error.expect("revert through a symlinked parent must be refused");
    assert!(err.message.contains("symlink"), "{}", err.message);
    assert_eq!(
        std::fs::read_to_string(outside.join("file.txt")).unwrap(),
        "outside data"
    );
    assert_eq!(
        std::fs::read_dir(&outside).unwrap().count(),
        1,
        "nothing may be created outside"
    );

    // The delete branch walks the same handle: a file that exists only in the
    // workspace behind a symlinked parent is never unlinked through it.
    let resp = revert(
        &session,
        serde_json::json!({"path": "sub/other.txt", "checkpoint": "cp-0"}),
    );
    assert!(resp.error.is_some());
    assert!(outside.join("file.txt").exists());
}
