//! Tests for `file_tools` (extracted from inline `mod tests`).

use super::*;
use crate::auto_snapshot::AutoSnapshotScheduler;
use std::path::PathBuf;
use std::time::Duration;

mod bounds;
mod containment;
mod output_format;
mod path_truncation;

fn setup() -> (tempfile::TempDir, PathBuf, AutoSnapshotScheduler) {
    let tmp = tempfile::tempdir().unwrap();
    let session = tmp.path().to_path_buf();
    std::fs::create_dir_all(session.join("workspace")).unwrap();
    std::fs::create_dir_all(session.join("system")).unwrap();
    std::fs::create_dir_all(session.join("auto_snapshots")).unwrap();
    let sched = AutoSnapshotScheduler::new(session.clone(), 10, 12, Duration::from_secs(300));
    (tmp, session, sched)
}

#[test]
fn tool_names_match_defs() {
    let defs = file_tool_defs();
    assert_eq!(defs.len(), FILE_TOOL_NAMES.len());
    for def in &defs {
        assert!(
            FILE_TOOL_NAMES.contains(&def.namespaced_name.as_str()),
            "def name {:?} not in FILE_TOOL_NAMES",
            def.namespaced_name,
        );
    }
}

#[test]
fn validate_path_rejects_traversal() {
    assert!(normalize_path("../etc/passwd").is_err());
    assert!(normalize_path("foo/../../bar").is_err());
}

#[test]
fn validate_path_rejects_absolute() {
    assert!(normalize_path("/etc/passwd").is_err());
}

#[test]
fn validate_path_rejects_empty() {
    assert!(normalize_path("").is_err());
}

#[test]
fn validate_path_rejects_null_bytes() {
    assert!(normalize_path("foo\0bar").is_err());
}

#[test]
fn validate_path_accepts_normal() {
    assert!(normalize_path("project/app.js").is_ok());
    assert!(normalize_path("a.txt").is_ok());
}

#[test]
fn normalize_path_strips_root_prefix() {
    assert_eq!(normalize_path("/root/hello.txt").unwrap(), "hello.txt");
    assert_eq!(normalize_path("/root/sub/file.py").unwrap(), "sub/file.py");
    assert_eq!(normalize_path("hello.txt").unwrap(), "hello.txt");
}

#[test]
fn parse_checkpoint_valid() {
    assert_eq!(parse_checkpoint("cp-0"), Ok(0));
    assert_eq!(parse_checkpoint("cp-11"), Ok(11));
}

#[test]
fn parse_checkpoint_invalid() {
    assert!(parse_checkpoint("0").is_err());
    assert!(parse_checkpoint("cp-").is_err());
    assert!(parse_checkpoint("cp-abc").is_err());
    assert!(parse_checkpoint("").is_err());
}

#[test]
fn list_changed_files_detects_created() {
    let (_tmp, session, mut sched) = setup();

    // Take baseline snapshot (empty workspace).
    sched.take_snapshot().unwrap();

    // Create a file after the snapshot.
    std::fs::write(session.join("workspace/new.txt"), "hello").unwrap();

    let workspace = session.join("workspace");
    let args = serde_json::json!({"format": "json"});
    let resp = handle_list_changed_files(&args, &sched, &workspace, Some(serde_json::json!(1)));
    let text = resp.result.unwrap()["content"][0]["text"].as_str().unwrap().to_string();
    let changes: Vec<Value> = serde_json::from_str(&text).unwrap();

    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0]["path"], "new.txt");
    assert_eq!(changes[0]["op"], "created");
}

#[test]
fn list_changed_files_detects_modified() {
    let (_tmp, session, mut sched) = setup();

    std::fs::write(session.join("workspace/file.txt"), "original").unwrap();
    sched.take_snapshot().unwrap();

    // Modify the file.
    std::fs::write(session.join("workspace/file.txt"), "modified content that is longer").unwrap();

    let workspace = session.join("workspace");
    let args = serde_json::json!({"format": "json"});
    let resp = handle_list_changed_files(&args, &sched, &workspace, Some(serde_json::json!(1)));
    let text = resp.result.unwrap()["content"][0]["text"].as_str().unwrap().to_string();
    let changes: Vec<Value> = serde_json::from_str(&text).unwrap();

    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0]["path"], "file.txt");
    assert_eq!(changes[0]["op"], "modified");
}

#[test]
fn list_changed_files_detects_deleted() {
    let (_tmp, session, mut sched) = setup();

    std::fs::write(session.join("workspace/gone.txt"), "bye").unwrap();
    sched.take_snapshot().unwrap();

    // Delete the file.
    std::fs::remove_file(session.join("workspace/gone.txt")).unwrap();

    let workspace = session.join("workspace");
    let args = serde_json::json!({"format": "json"});
    let resp = handle_list_changed_files(&args, &sched, &workspace, Some(serde_json::json!(1)));
    let text = resp.result.unwrap()["content"][0]["text"].as_str().unwrap().to_string();
    let changes: Vec<Value> = serde_json::from_str(&text).unwrap();

    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0]["path"], "gone.txt");
    assert_eq!(changes[0]["op"], "deleted");
}

/// Roundtrip test: write a file, snapshot, copy it, delete original,
/// revert via snapshots_revert, verify content matches exactly.
#[test]
fn revert_file_roundtrip_content_preserved() {
    let (_tmp, session, mut sched) = setup();

    // Write a file with known content.
    let content = "The quick brown fox jumps over the lazy dog.\nLine 2.\n";
    std::fs::write(session.join("workspace/important.txt"), content).unwrap();

    // Take a snapshot.
    sched.take_snapshot().unwrap();

    // Copy the file (proving we can read it).
    let copied = std::fs::read_to_string(session.join("workspace/important.txt")).unwrap();
    assert_eq!(copied, content);

    // Delete the original.
    std::fs::remove_file(session.join("workspace/important.txt")).unwrap();
    assert!(!session.join("workspace/important.txt").exists());

    // Revert via snapshots_revert.
    let args = serde_json::json!({"path": "important.txt", "checkpoint": "cp-0"});
    let resp = handle_revert_file(
        &args,
        &sched,
        &session.join("workspace"),
        Some(serde_json::json!(1)),
        None,
    );

    // Verify success with action and checkpoint fields.
    assert!(resp.error.is_none(), "snapshots_revert failed: {:?}", resp.error);
    let result_text = resp.result.unwrap()["content"][0]["text"].as_str().unwrap().to_string();
    let result: Value = serde_json::from_str(&result_text).unwrap();
    assert_eq!(result["reverted"], true);
    assert_eq!(result["action"], "restored");
    assert_eq!(result["checkpoint"], "cp-0");

    // Verify the file is back with exact same content.
    let recovered = std::fs::read_to_string(session.join("workspace/important.txt")).unwrap();
    assert_eq!(recovered, content, "recovered content must match original exactly");
}

#[tokio::test]
async fn revert_file_security_event_emits_from_async_runtime() {
    let (_tmp, session, mut sched) = setup();

    std::fs::write(session.join("workspace/important.txt"), "baseline").unwrap();
    sched.take_snapshot().unwrap();
    std::fs::write(session.join("workspace/important.txt"), "changed").unwrap();

    let args = serde_json::json!({"path": "important.txt", "checkpoint": "cp-0"});
    let (resp, file_event) =
        handle_revert_file_with_security_event(&args, &sched, &session.join("workspace"), Some(serde_json::json!(1)));

    assert!(resp.error.is_none());
    let file_event = file_event.expect("successful revert must produce file event");
    assert_eq!(file_event.action, capsem_logger::FileAction::Restored);
    assert_eq!(file_event.path, "important.txt (from cp-0)");

    let db_path = session.join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    let rules = crate::net::policy_config::SecurityRuleSet::new(Vec::new());
    let event_id = crate::security_engine::emit_file_security_write_and_rules(&writer, &rules, file_event)
        .await
        .expect("async file event emit must produce event id");
    writer.shutdown_blocking();

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let row: (String, String, String) = conn
        .query_row("SELECT event_id, action, path FROM fs_events", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .unwrap();
    assert_eq!(row.0, event_id.as_str());
    assert_eq!(row.0.len(), 12);
    assert_eq!(row.1, "restored");
    assert_eq!(row.2, "important.txt (from cp-0)");
}

#[test]
fn revert_file_deletes_created_file() {
    let (_tmp, session, mut sched) = setup();

    // Snapshot with empty workspace.
    sched.take_snapshot().unwrap();

    // Create a new file.
    std::fs::write(session.join("workspace/new.txt"), "should be deleted").unwrap();

    // Revert -- file didn't exist in snapshot, so it should be deleted.
    let args = serde_json::json!({"path": "new.txt", "checkpoint": "cp-0"});
    let resp = handle_revert_file(
        &args,
        &sched,
        &session.join("workspace"),
        Some(serde_json::json!(1)),
        None,
    );

    assert!(resp.error.is_none());
    assert!(!session.join("workspace/new.txt").exists());

    // Verify action and checkpoint in response.
    let result_text = resp.result.unwrap()["content"][0]["text"].as_str().unwrap().to_string();
    let result: Value = serde_json::from_str(&result_text).unwrap();
    assert_eq!(result["action"], "deleted");
    assert_eq!(result["checkpoint"], "cp-0");
}

#[cfg(unix)]
#[test]
fn revert_file_rejects_snapshot_parent_symlink_escape() {
    let (tmp, session, mut sched) = setup();
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.txt"), "external secret").unwrap();

    sched.take_snapshot().unwrap();
    std::os::unix::fs::symlink(&outside, session.join("auto_snapshots/0/workspace/escape")).unwrap();

    let args = serde_json::json!({"path": "escape/secret.txt", "checkpoint": "cp-0"});
    let resp = handle_revert_file(
        &args,
        &sched,
        &session.join("workspace"),
        Some(serde_json::json!(1)),
        None,
    );

    let err = resp.error.expect("symlink escape must be rejected");
    assert!(
        err.message.contains("snapshot source parent contains symlink"),
        "unexpected error: {}",
        err.message
    );
    assert!(
        !session.join("workspace/escape").exists(),
        "restore must not materialize escaped snapshot content into workspace"
    );
}

#[cfg(unix)]
#[test]
fn revert_file_replaces_live_final_symlink_without_touching_target() {
    let (tmp, session, mut sched) = setup();
    let outside = tmp.path().join("outside.txt");
    std::fs::write(&outside, "outside secret").unwrap();
    std::fs::write(session.join("workspace/safe.txt"), "snapshot data").unwrap();
    sched.take_snapshot().unwrap();

    std::fs::remove_file(session.join("workspace/safe.txt")).unwrap();
    std::os::unix::fs::symlink(&outside, session.join("workspace/safe.txt")).unwrap();

    let args = serde_json::json!({"path": "safe.txt", "checkpoint": "cp-0"});
    let resp = handle_revert_file(
        &args,
        &sched,
        &session.join("workspace"),
        Some(serde_json::json!(1)),
        None,
    );

    assert!(resp.error.is_none(), "restore failed: {:?}", resp.error);
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "outside secret");
    assert!(
        !session
            .join("workspace/safe.txt")
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink(),
        "workspace file should be restored as a regular file"
    );
    assert_eq!(
        std::fs::read_to_string(session.join("workspace/safe.txt")).unwrap(),
        "snapshot data"
    );
}

#[cfg(unix)]
#[test]
fn revert_file_restores_snapshot_symlink_without_pulling_target_bytes() {
    let (tmp, session, mut sched) = setup();
    let outside = tmp.path().join("outside.txt");
    std::fs::write(&outside, "outside secret").unwrap();
    std::os::unix::fs::symlink(&outside, session.join("workspace/link.txt")).unwrap();
    sched.take_snapshot().unwrap();

    std::fs::remove_file(session.join("workspace/link.txt")).unwrap();
    let args = serde_json::json!({"path": "link.txt", "checkpoint": "cp-0"});
    let resp = handle_revert_file(
        &args,
        &sched,
        &session.join("workspace"),
        Some(serde_json::json!(1)),
        None,
    );

    assert!(resp.error.is_none(), "restore failed: {:?}", resp.error);
    let restored = session.join("workspace/link.txt");
    assert!(
        restored.symlink_metadata().unwrap().file_type().is_symlink(),
        "snapshot symlink should remain a symlink, not copied target bytes"
    );
    assert_eq!(std::fs::read_link(restored).unwrap(), outside);
}

#[test]
fn revert_file_rejects_path_traversal() {
    let (_tmp, session, mut sched) = setup();
    sched.take_snapshot().unwrap();

    let args = serde_json::json!({"path": "../../../etc/passwd", "checkpoint": "cp-0"});
    let resp = handle_revert_file(
        &args,
        &sched,
        &session.join("workspace"),
        Some(serde_json::json!(1)),
        None,
    );
    assert!(resp.error.is_some());
}

#[test]
fn revert_file_rejects_invalid_checkpoint() {
    let (_tmp, session, mut sched) = setup();
    sched.take_snapshot().unwrap();

    let args = serde_json::json!({"path": "file.txt", "checkpoint": "bad"});
    let resp = handle_revert_file(
        &args,
        &sched,
        &session.join("workspace"),
        Some(serde_json::json!(1)),
        None,
    );
    assert!(resp.error.is_some());
}

#[test]
fn revert_file_rejects_nonexistent_checkpoint() {
    let (_tmp, session, sched) = setup();
    // No snapshots taken.
    let args = serde_json::json!({"path": "file.txt", "checkpoint": "cp-0"});
    let resp = handle_revert_file(
        &args,
        &sched,
        &session.join("workspace"),
        Some(serde_json::json!(1)),
        None,
    );
    assert!(resp.error.is_some());
}

/// File changed 3 times, snapshot after each change, revert all 3 to their
/// respective checkpoint, verify each recovered content matches exactly.
#[test]
fn revert_three_versions_of_same_file() {
    let (_tmp, session, mut sched) = setup();
    let ws = session.join("workspace");
    let file = ws.join("evolving.txt");

    // Version 1
    std::fs::write(&file, "version ONE").unwrap();
    sched.take_snapshot().unwrap(); // cp-0

    // Version 2
    std::fs::write(&file, "version TWO -- longer content here").unwrap();
    sched.take_snapshot().unwrap(); // cp-1

    // Version 3
    std::fs::write(&file, "version THREE!!!").unwrap();
    sched.take_snapshot().unwrap(); // cp-2

    // Overwrite with garbage
    std::fs::write(&file, "CORRUPTED").unwrap();

    // Revert to version 1 (cp-0)
    let args = serde_json::json!({"path": "evolving.txt", "checkpoint": "cp-0"});
    let resp = handle_revert_file(&args, &sched, &ws, Some(serde_json::json!(1)), None);
    assert!(resp.error.is_none(), "revert to cp-0 failed: {:?}", resp.error);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "version ONE");
    let result_text = resp.result.unwrap()["content"][0]["text"].as_str().unwrap().to_string();
    let result: Value = serde_json::from_str(&result_text).unwrap();
    assert_eq!(result["action"], "restored");
    assert_eq!(result["checkpoint"], "cp-0");

    // Revert to version 2 (cp-1)
    let args = serde_json::json!({"path": "evolving.txt", "checkpoint": "cp-1"});
    let resp = handle_revert_file(&args, &sched, &ws, Some(serde_json::json!(1)), None);
    assert!(resp.error.is_none(), "revert to cp-1 failed: {:?}", resp.error);
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "version TWO -- longer content here"
    );

    // Revert to version 3 (cp-2)
    let args = serde_json::json!({"path": "evolving.txt", "checkpoint": "cp-2"});
    let resp = handle_revert_file(&args, &sched, &ws, Some(serde_json::json!(1)), None);
    assert!(resp.error.is_none(), "revert to cp-2 failed: {:?}", resp.error);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "version THREE!!!");
}

/// File deleted after snapshot, then recovered via revert, content matches.
#[test]
fn delete_then_recover_via_revert() {
    let (_tmp, session, mut sched) = setup();
    let ws = session.join("workspace");
    let file = ws.join("precious.txt");

    let content = "This file is very important.\nDo not delete.\n";
    std::fs::write(&file, content).unwrap();
    sched.take_snapshot().unwrap(); // cp-0

    // Copy it (proving we can read it).
    let copied = std::fs::read_to_string(&file).unwrap();
    assert_eq!(copied, content);

    // Delete
    std::fs::remove_file(&file).unwrap();
    assert!(!file.exists());

    // Recover
    let args = serde_json::json!({"path": "precious.txt", "checkpoint": "cp-0"});
    let resp = handle_revert_file(&args, &sched, &ws, Some(serde_json::json!(1)), None);
    assert!(resp.error.is_none());

    // Verify exact content
    let recovered = std::fs::read_to_string(&file).unwrap();
    assert_eq!(recovered, content, "recovered content must match original exactly");

    // Verify response fields
    let result_text = resp.result.unwrap()["content"][0]["text"].as_str().unwrap().to_string();
    let result: Value = serde_json::from_str(&result_text).unwrap();
    assert_eq!(result["action"], "restored");
    assert_eq!(result["checkpoint"], "cp-0");
}

/// list_changed_files shows all 3 file operations across snapshots.
#[test]
fn list_changed_files_shows_create_modify_delete() {
    let (_tmp, session, mut sched) = setup();
    let ws = session.join("workspace");

    // Create 3 files, snapshot.
    std::fs::write(ws.join("keep.txt"), "original").unwrap();
    std::fs::write(ws.join("modify_me.txt"), "before").unwrap();
    std::fs::write(ws.join("delete_me.txt"), "goodbye").unwrap();
    sched.take_snapshot().unwrap(); // cp-0

    // Modify one, delete one, create a new one.
    std::fs::write(ws.join("modify_me.txt"), "after!").unwrap();
    std::fs::remove_file(ws.join("delete_me.txt")).unwrap();
    std::fs::write(ws.join("brand_new.txt"), "hello").unwrap();

    let args = serde_json::json!({"format": "json"});
    let resp = handle_list_changed_files(&args, &sched, &ws, Some(serde_json::json!(1)));
    let text = resp.result.unwrap()["content"][0]["text"].as_str().unwrap().to_string();
    let changes: Vec<Value> = serde_json::from_str(&text).unwrap();

    // Should see: brand_new.txt (created), modify_me.txt (modified), delete_me.txt (deleted)
    // keep.txt should NOT appear (unchanged).
    let paths: Vec<&str> = changes.iter().map(|c| c["path"].as_str().unwrap()).collect();
    assert!(paths.contains(&"brand_new.txt"), "missing created file: {paths:?}");
    assert!(paths.contains(&"modify_me.txt"), "missing modified file: {paths:?}");
    assert!(paths.contains(&"delete_me.txt"), "missing deleted file: {paths:?}");
    assert!(
        !paths.contains(&"keep.txt"),
        "unchanged file should not appear: {paths:?}"
    );

    // Verify ops
    let get_op = |name: &str| -> &str {
        changes.iter().find(|c| c["path"] == name).unwrap()["op"]
            .as_str()
            .unwrap()
    };
    assert_eq!(get_op("brand_new.txt"), "created");
    assert_eq!(get_op("modify_me.txt"), "modified");
    assert_eq!(get_op("delete_me.txt"), "deleted");
}

/// snapshots_list defaults to compact per-snapshot change counts.
#[test]
fn list_snapshots_changes_vs_previous() {
    let (_tmp, session, mut sched) = setup();
    let ws = session.join("workspace");

    // Create a file and snapshot.
    std::fs::write(ws.join("hello.txt"), "world").unwrap();
    sched.take_snapshot().unwrap(); // cp-0

    // Modify the file and snapshot again.
    std::fs::write(ws.join("hello.txt"), "modified world content").unwrap();
    sched.take_snapshot().unwrap(); // cp-1

    let args = serde_json::json!({"format": "json"});
    let resp = handle_list_snapshots(&args, &sched, &ws, Some(serde_json::json!(1)));
    let text = resp.result.unwrap()["content"][0]["text"].as_str().unwrap().to_string();
    let summary: Value = serde_json::from_str(&text).unwrap();
    let entries = summary["snapshots"].as_array().unwrap();

    assert_eq!(entries.len(), 2);
    // Newest first: cp-1, cp-0. Full changes are intentionally omitted by
    // default so snapshot internals do not bleed into generic consumers.
    assert!(entries[0]["changes"].is_null(), "full changes require opt-in");
    assert!(entries[1]["changes"].is_null(), "full changes require opt-in");

    // cp-0: hello.txt is "new" (rendered as created in the summary).
    assert_eq!(entries[1]["changes_summary"]["created"], 1);
    assert_eq!(entries[1]["changes_summary"]["edited"], 0);
    assert_eq!(entries[1]["changes_summary"]["deleted"], 0);
    assert_eq!(entries[1]["changes_summary"]["total"], 1);

    // cp-1: hello.txt is "modified" (rendered as edited in the summary).
    assert_eq!(entries[0]["changes_summary"]["created"], 0);
    assert_eq!(entries[0]["changes_summary"]["edited"], 1);
    assert_eq!(entries[0]["changes_summary"]["deleted"], 0);
    assert_eq!(entries[0]["changes_summary"]["total"], 1);
}

/// Explicit MCP callers can request full per-file snapshot changes.
#[test]
fn list_snapshots_include_changes_is_explicit() {
    let (_tmp, session, mut sched) = setup();
    let ws = session.join("workspace");

    std::fs::write(ws.join("hello.txt"), "world").unwrap();
    sched.take_snapshot().unwrap(); // cp-0
    std::fs::write(ws.join("hello.txt"), "modified world content").unwrap();
    sched.take_snapshot().unwrap(); // cp-1

    let args = serde_json::json!({"format": "json", "include_changes": true});
    let resp = handle_list_snapshots(&args, &sched, &ws, Some(serde_json::json!(1)));
    let text = extract_text(&resp);
    let summary: Value = serde_json::from_str(&text).unwrap();
    let entries = summary["snapshots"].as_array().unwrap();

    assert_eq!(entries.len(), 2);
    let cp1_changes = entries[0]["changes"].as_array().unwrap();
    let cp0_changes = entries[1]["changes"].as_array().unwrap();

    assert_eq!(cp0_changes.len(), 1);
    assert_eq!(cp0_changes[0]["path"], "hello.txt");
    assert_eq!(cp0_changes[0]["op"], "new");
    assert_eq!(cp1_changes.len(), 1);
    assert_eq!(cp1_changes[0]["path"], "hello.txt");
    assert_eq!(cp1_changes[0]["op"], "modified");
}

/// All snapshots are shown (no empty filtering).
#[test]
fn list_snapshots_shows_all() {
    let (_tmp, session, mut sched) = setup();
    let ws = session.join("workspace");

    // Snapshot with empty workspace.
    sched.take_snapshot().unwrap(); // cp-0 (empty)

    // Create a file and snapshot again.
    std::fs::write(ws.join("data.txt"), "content").unwrap();
    sched.take_snapshot().unwrap(); // cp-1

    let args = serde_json::json!({"format": "json"});
    let resp = handle_list_snapshots(&args, &sched, &ws, Some(serde_json::json!(1)));
    let text = resp.result.unwrap()["content"][0]["text"].as_str().unwrap().to_string();
    let summary: Value = serde_json::from_str(&text).unwrap();
    let entries = summary["snapshots"].as_array().unwrap();

    // Both should be present (no filtering).
    assert_eq!(entries.len(), 2);
}

/// snapshots_revert auto-selects newest snapshot containing the file when
/// checkpoint is omitted.
#[test]
fn revert_file_auto_selects_checkpoint() {
    let (_tmp, session, mut sched) = setup();
    let ws = session.join("workspace");

    // Create a file and take two snapshots.
    std::fs::write(ws.join("auto.txt"), "version 1").unwrap();
    sched.take_snapshot().unwrap(); // cp-0

    std::fs::write(ws.join("auto.txt"), "version 2 is longer").unwrap();
    sched.take_snapshot().unwrap(); // cp-1

    // Now corrupt the file.
    std::fs::write(ws.join("auto.txt"), "CORRUPTED").unwrap();

    // Revert without specifying checkpoint -- should pick cp-1 (newest).
    let args = serde_json::json!({"path": "auto.txt"});
    let resp = handle_revert_file(&args, &sched, &ws, Some(serde_json::json!(1)), None);
    assert!(resp.error.is_none(), "auto-select revert failed: {:?}", resp.error);

    let result_text = resp.result.unwrap()["content"][0]["text"].as_str().unwrap().to_string();
    let result: Value = serde_json::from_str(&result_text).unwrap();
    assert_eq!(result["action"], "restored");
    assert_eq!(result["checkpoint"], "cp-1");
    assert_eq!(
        std::fs::read_to_string(ws.join("auto.txt")).unwrap(),
        "version 2 is longer"
    );
}

/// snapshots_revert errors when no snapshot contains the file and checkpoint
/// is omitted.
#[test]
fn revert_file_auto_select_no_match() {
    let (_tmp, session, mut sched) = setup();
    let ws = session.join("workspace");

    // Snapshot empty workspace.
    sched.take_snapshot().unwrap();

    // Create a file that doesn't exist in any snapshot.
    std::fs::write(ws.join("orphan.txt"), "data").unwrap();

    let args = serde_json::json!({"path": "orphan.txt"});
    let resp = handle_revert_file(&args, &sched, &ws, Some(serde_json::json!(1)), None);
    assert!(resp.error.is_some());
    let err_msg = &resp.error.unwrap().message;
    assert!(
        err_msg.contains("no snapshot contains this file"),
        "unexpected error: {err_msg}"
    );
}

// -- Pagination and text table tests (TDD: written before implementation) --

// -------------------------------------------------------------------
// Symlink handling in collect_files
// -------------------------------------------------------------------

#[test]
fn collect_files_includes_symlinks() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("real.txt"), "data").unwrap();
    std::os::unix::fs::symlink("real.txt", dir.path().join("link.txt")).unwrap();

    let files = collect_files(dir.path());
    assert!(files.contains_key("real.txt"), "regular file must appear");
    assert!(files.contains_key("link.txt"), "symlink must appear");
    assert_eq!(files.len(), 2);
    assert!(!files["real.txt"].is_symlink);
    assert!(files["link.txt"].is_symlink);
}

#[test]
fn collect_files_does_not_follow_symlinks_for_size() {
    let dir = tempfile::tempdir().unwrap();
    let data = "x".repeat(1000);
    std::fs::write(dir.path().join("big.txt"), &data).unwrap();
    std::os::unix::fs::symlink("big.txt", dir.path().join("link.txt")).unwrap();

    let files = collect_files(dir.path());
    let link_size = files["link.txt"].size;
    // Symlink size is the length of the target path, not the target file size.
    // "big.txt" is 7 bytes as a symlink target.
    assert!(
        link_size < 100,
        "symlink size should be small (target path), not {link_size}"
    );
}

// -- no-follow write path (revert TOCTOU) --

#[cfg(unix)]
#[test]
fn write_regular_file_no_follow_refuses_existing_symlink() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("outside.txt");
    std::fs::write(&target, b"original").unwrap();
    let link = dir.path().join("workspace_file");
    symlink(&target, &link).unwrap();

    // A guest raced a symlink into the workspace target between the containment
    // check and the write. The write must refuse, never follow it.
    let result = write_regular_file_no_follow(&link, b"attacker", 0o644);
    assert!(result.is_err(), "must refuse to write through a pre-existing symlink");
    assert_eq!(
        std::fs::read(&target).unwrap(),
        b"original",
        "the symlink target outside the workspace must be untouched"
    );
}

#[cfg(unix)]
#[test]
fn write_regular_file_no_follow_creates_fresh_file_with_mode() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("restored");
    write_regular_file_no_follow(&path, b"hi", 0o600).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"hi");
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
}

// APFS rejects invalid UTF-8 names at creation; Linux filesystems admit them.
#[cfg(target_os = "linux")]
#[test]
fn collect_files_does_not_collapse_distinct_non_utf8_names() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let dir = tempfile::tempdir().unwrap();
    // Two distinct invalid-UTF8 filenames. to_string_lossy maps both to the
    // same replacement string, collapsing them in the map and corrupting change
    // detection. They are unaddressable through the JSON tool API regardless.
    std::fs::write(dir.path().join(OsStr::from_bytes(b"\xff")), b"a").unwrap();
    std::fs::write(dir.path().join(OsStr::from_bytes(b"\xfe")), b"b").unwrap();

    let files = collect_files(dir.path());
    assert_eq!(
        files.len(),
        0,
        "distinct non-UTF8 filenames must not collapse into a single entry: {files:?}"
    );
}

// Shared helper for this module and its submodules.

/// Helper to extract the text content from a JsonRpcResponse.
fn extract_text(resp: &JsonRpcResponse) -> String {
    resp.result.as_ref().unwrap()["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string()
}
