//! Tests for `file_tools` (extracted from inline `mod tests`).

use super::*;
use crate::auto_snapshot::AutoSnapshotScheduler;
use std::path::PathBuf;
use std::time::Duration;

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
    let text = resp.result.unwrap()["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string();
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
    std::fs::write(
        session.join("workspace/file.txt"),
        "modified content that is longer",
    )
    .unwrap();

    let workspace = session.join("workspace");
    let args = serde_json::json!({"format": "json"});
    let resp = handle_list_changed_files(&args, &sched, &workspace, Some(serde_json::json!(1)));
    let text = resp.result.unwrap()["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string();
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
    let text = resp.result.unwrap()["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string();
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
    assert!(
        resp.error.is_none(),
        "snapshots_revert failed: {:?}",
        resp.error
    );
    let result_text = resp.result.unwrap()["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string();
    let result: Value = serde_json::from_str(&result_text).unwrap();
    assert_eq!(result["reverted"], true);
    assert_eq!(result["action"], "restored");
    assert_eq!(result["checkpoint"], "cp-0");

    // Verify the file is back with exact same content.
    let recovered = std::fs::read_to_string(session.join("workspace/important.txt")).unwrap();
    assert_eq!(
        recovered, content,
        "recovered content must match original exactly"
    );
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
    let result_text = resp.result.unwrap()["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string();
    let result: Value = serde_json::from_str(&result_text).unwrap();
    assert_eq!(result["action"], "deleted");
    assert_eq!(result["checkpoint"], "cp-0");
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
    assert!(
        resp.error.is_none(),
        "revert to cp-0 failed: {:?}",
        resp.error
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "version ONE");
    let result_text = resp.result.unwrap()["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string();
    let result: Value = serde_json::from_str(&result_text).unwrap();
    assert_eq!(result["action"], "restored");
    assert_eq!(result["checkpoint"], "cp-0");

    // Revert to version 2 (cp-1)
    let args = serde_json::json!({"path": "evolving.txt", "checkpoint": "cp-1"});
    let resp = handle_revert_file(&args, &sched, &ws, Some(serde_json::json!(1)), None);
    assert!(
        resp.error.is_none(),
        "revert to cp-1 failed: {:?}",
        resp.error
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "version TWO -- longer content here"
    );

    // Revert to version 3 (cp-2)
    let args = serde_json::json!({"path": "evolving.txt", "checkpoint": "cp-2"});
    let resp = handle_revert_file(&args, &sched, &ws, Some(serde_json::json!(1)), None);
    assert!(
        resp.error.is_none(),
        "revert to cp-2 failed: {:?}",
        resp.error
    );
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
    assert_eq!(
        recovered, content,
        "recovered content must match original exactly"
    );

    // Verify response fields
    let result_text = resp.result.unwrap()["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string();
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
    std::fs::write(ws.join("modify_me.txt"), "after -- different length").unwrap();
    std::fs::remove_file(ws.join("delete_me.txt")).unwrap();
    std::fs::write(ws.join("brand_new.txt"), "hello").unwrap();

    let args = serde_json::json!({"format": "json"});
    let resp = handle_list_changed_files(&args, &sched, &ws, Some(serde_json::json!(1)));
    let text = resp.result.unwrap()["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string();
    let changes: Vec<Value> = serde_json::from_str(&text).unwrap();

    // Should see: brand_new.txt (created), modify_me.txt (modified), delete_me.txt (deleted)
    // keep.txt should NOT appear (unchanged).
    let paths: Vec<&str> = changes
        .iter()
        .map(|c| c["path"].as_str().unwrap())
        .collect();
    assert!(
        paths.contains(&"brand_new.txt"),
        "missing created file: {paths:?}"
    );
    assert!(
        paths.contains(&"modify_me.txt"),
        "missing modified file: {paths:?}"
    );
    assert!(
        paths.contains(&"delete_me.txt"),
        "missing deleted file: {paths:?}"
    );
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

/// snapshots_list includes per-snapshot changes and filters empty snapshots.
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
    let text = resp.result.unwrap()["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string();
    let summary: Value = serde_json::from_str(&text).unwrap();
    let entries = summary["snapshots"].as_array().unwrap();

    assert_eq!(entries.len(), 2);
    // Newest first: cp-1, cp-0
    let cp1_changes = entries[0]["changes"].as_array().unwrap();
    let cp0_changes = entries[1]["changes"].as_array().unwrap();

    // cp-0: hello.txt is "new" (didn't exist before)
    assert_eq!(cp0_changes.len(), 1);
    assert_eq!(cp0_changes[0]["path"], "hello.txt");
    assert_eq!(cp0_changes[0]["op"], "new");

    // cp-1: hello.txt is "modified" (changed since cp-0)
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
    let text = resp.result.unwrap()["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string();
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
    assert!(
        resp.error.is_none(),
        "auto-select revert failed: {:?}",
        resp.error
    );

    let result_text = resp.result.unwrap()["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string();
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

/// Helper to extract the text content from a JsonRpcResponse.
fn extract_text(resp: &JsonRpcResponse) -> String {
    resp.result.as_ref().unwrap()["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn changes_returns_text_table() {
    let (_tmp, session, mut sched) = setup();
    let ws = session.join("workspace");

    std::fs::write(ws.join("hello.txt"), "world").unwrap();
    sched.take_snapshot().unwrap();
    std::fs::write(ws.join("new.txt"), "created").unwrap();

    let args = serde_json::json!({});
    let resp = handle_list_changed_files(&args, &sched, &ws, Some(serde_json::json!(1)));
    let text = extract_text(&resp);

    // Default format is text table, not JSON.
    assert!(
        serde_json::from_str::<Vec<Value>>(&text).is_err(),
        "default response should NOT be a JSON array"
    );
    assert!(text.contains("Changed Files"), "missing header: {text}");
    assert!(text.contains("Path"), "missing Path column: {text}");
    assert!(text.contains("Op"), "missing Op column: {text}");
    assert!(text.contains("new.txt"), "missing file entry: {text}");
    assert!(text.contains("created"), "missing op value: {text}");
}

#[test]
fn changes_pagination_truncates_large_output() {
    let (_tmp, session, mut sched) = setup();
    let ws = session.join("workspace");

    // Take empty snapshot, then create 300 files.
    sched.take_snapshot().unwrap();
    for i in 0..300 {
        std::fs::write(ws.join(format!("file_{i:04}.txt")), format!("content {i}")).unwrap();
    }

    let args = serde_json::json!({});
    let resp = handle_list_changed_files(&args, &sched, &ws, Some(serde_json::json!(1)));
    let text = extract_text(&resp);

    // Response should be bounded by DEFAULT_MAX_LENGTH + header overhead.
    let max_allowed = super::super::builtin_tools::DEFAULT_MAX_LENGTH as usize + 500;
    assert!(
        text.len() <= max_allowed,
        "response too large: {} chars (max {})",
        text.len(),
        max_allowed
    );
    // Should indicate pagination is available.
    assert!(
        text.contains("start_index="),
        "missing pagination hint: {text}"
    );
}

#[test]
fn changes_pagination_continuation() {
    let (_tmp, session, mut sched) = setup();
    let ws = session.join("workspace");

    sched.take_snapshot().unwrap();
    for i in 0..300 {
        std::fs::write(ws.join(format!("file_{i:04}.txt")), format!("content {i}")).unwrap();
    }

    // First page.
    let args = serde_json::json!({});
    let resp = handle_list_changed_files(&args, &sched, &ws, Some(serde_json::json!(1)));
    let page1 = extract_text(&resp);

    // Extract start_index from pagination hint.
    let idx_str = page1
        .split("start_index=")
        .nth(1)
        .unwrap()
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .unwrap();
    let next_start: u64 = idx_str.parse().unwrap();

    // Second page.
    let args = serde_json::json!({"start_index": next_start});
    let resp = handle_list_changed_files(&args, &sched, &ws, Some(serde_json::json!(1)));
    let page2 = extract_text(&resp);

    // Pages should have different content.
    assert_ne!(page1, page2, "pages should differ");
    // Page 2 should not re-include the header.
    assert!(
        !page2.starts_with("Changed Files"),
        "page 2 should not repeat the header"
    );
}

#[test]
fn changes_custom_max_length() {
    let (_tmp, session, mut sched) = setup();
    let ws = session.join("workspace");

    sched.take_snapshot().unwrap();
    for i in 0..20 {
        std::fs::write(ws.join(format!("f_{i}.txt")), "x").unwrap();
    }

    let args = serde_json::json!({"max_length": 200});
    let resp = handle_list_changed_files(&args, &sched, &ws, Some(serde_json::json!(1)));
    let text = extract_text(&resp);

    // Header + chunk: allow some overhead for the pagination hint itself.
    assert!(
        text.len() <= 500,
        "response should be short with max_length=200, got {} chars",
        text.len()
    );
    assert!(
        text.contains("start_index="),
        "should paginate at max_length=200"
    );
}

#[test]
fn changes_small_result_no_pagination() {
    let (_tmp, session, mut sched) = setup();
    let ws = session.join("workspace");

    sched.take_snapshot().unwrap();
    std::fs::write(ws.join("only.txt"), "small").unwrap();

    let args = serde_json::json!({});
    let resp = handle_list_changed_files(&args, &sched, &ws, Some(serde_json::json!(1)));
    let text = extract_text(&resp);

    assert!(
        !text.contains("start_index="),
        "should not paginate small results: {text}"
    );
    assert!(text.contains("only.txt"), "missing file entry: {text}");
}

#[test]
fn changes_format_json_returns_raw() {
    let (_tmp, session, mut sched) = setup();
    let ws = session.join("workspace");

    std::fs::write(ws.join("a.txt"), "original").unwrap();
    sched.take_snapshot().unwrap();
    std::fs::write(ws.join("b.txt"), "new").unwrap();

    let args = serde_json::json!({"format": "json"});
    let resp = handle_list_changed_files(&args, &sched, &ws, Some(serde_json::json!(1)));
    let text = extract_text(&resp);

    // format=json should return valid JSON array.
    let changes: Vec<Value> =
        serde_json::from_str(&text).expect("format=json should return valid JSON array");
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0]["path"], "b.txt");
    assert_eq!(changes[0]["op"], "created");
}

#[test]
fn list_returns_text_table() {
    let (_tmp, session, mut sched) = setup();
    let ws = session.join("workspace");

    std::fs::write(ws.join("hello.txt"), "world").unwrap();
    sched.take_snapshot().unwrap();
    std::fs::write(ws.join("hello.txt"), "modified world content").unwrap();
    sched.take_snapshot().unwrap();

    let args = serde_json::json!({});
    let resp = handle_list_snapshots(&args, &sched, &ws, Some(serde_json::json!(1)));
    let text = extract_text(&resp);

    // Default format is text table, not JSON.
    assert!(
        serde_json::from_str::<Value>(&text).is_err(),
        "default response should NOT be JSON"
    );
    assert!(text.contains("Snapshots"), "missing header: {text}");
    assert!(
        text.contains("Checkpoint"),
        "missing Checkpoint column: {text}"
    );
    // Changes should use compact format.
    assert!(
        text.contains('+') || text.contains('~'),
        "changes should use compact +/~ format: {text}"
    );
}

#[test]
fn list_pagination_works() {
    let (_tmp, session, mut sched) = setup();
    let ws = session.join("workspace");

    // Create many snapshots with files to generate a large response.
    for i in 0..8 {
        for j in 0..20 {
            std::fs::write(ws.join(format!("f_{i}_{j}.txt")), format!("{i}{j}")).unwrap();
        }
        sched.take_snapshot().unwrap();
    }

    let args = serde_json::json!({"max_length": 500});
    let resp = handle_list_snapshots(&args, &sched, &ws, Some(serde_json::json!(1)));
    let text = extract_text(&resp);

    assert!(
        text.len() <= 1000,
        "response should respect max_length, got {} chars",
        text.len()
    );
    assert!(text.contains("start_index="), "should paginate: {text}");
}

#[test]
fn list_format_json_returns_raw() {
    let (_tmp, session, mut sched) = setup();
    let ws = session.join("workspace");

    std::fs::write(ws.join("a.txt"), "data").unwrap();
    sched.take_snapshot().unwrap();

    let args = serde_json::json!({"format": "json"});
    let resp = handle_list_snapshots(&args, &sched, &ws, Some(serde_json::json!(1)));
    let text = extract_text(&resp);

    // format=json should return valid JSON.
    let summary: Value = serde_json::from_str(&text).expect("format=json should return valid JSON");
    assert!(summary["snapshots"].is_array());
}

/// Contract test: verifies the exact response shape the frontend depends on.
///
/// The frontend (api.ts:listSnapshots) calls callMcpTool('snapshots_list', {format:'json'})
/// and parses result.content[0].text as JSON expecting these fields. If this test
/// breaks, the snapshot panel will break too.
#[test]
fn list_format_json_frontend_contract() {
    let (_tmp, session, mut sched) = setup();
    let ws = session.join("workspace");

    std::fs::write(ws.join("hello.txt"), "world").unwrap();
    sched.take_snapshot().unwrap();
    std::fs::write(ws.join("hello.txt"), "changed").unwrap();
    sched.take_snapshot().unwrap();

    // Frontend always passes format: "json".
    let args = serde_json::json!({"format": "json"});
    let resp = handle_list_snapshots(&args, &sched, &ws, Some(serde_json::json!(1)));

    // Response must have result.content[0].text.
    let result = resp.result.as_ref().expect("response must have result");
    let content = result["content"]
        .as_array()
        .expect("result must have content array");
    assert!(!content.is_empty(), "content must not be empty");
    let text = content[0]["text"]
        .as_str()
        .expect("content[0] must have text string");

    // text must be valid JSON with the expected shape.
    let data: Value =
        serde_json::from_str(text).expect("content text must be valid JSON when format=json");

    // Top-level fields the frontend depends on.
    assert!(data["snapshots"].is_array(), "must have snapshots array");
    assert!(data["auto_max"].is_number(), "must have auto_max number");
    assert!(
        data["manual_max"].is_number(),
        "must have manual_max number"
    );
    assert!(
        data["manual_available"].is_number(),
        "must have manual_available number"
    );

    // Each snapshot must have the fields SnapshotsTab.svelte reads.
    let snaps = data["snapshots"].as_array().unwrap();
    assert!(snaps.len() >= 2, "should have at least 2 snapshots");
    for snap in snaps {
        assert!(
            snap["checkpoint"].is_string(),
            "snapshot must have checkpoint: {snap}"
        );
        assert!(snap["slot"].is_number(), "snapshot must have slot: {snap}");
        assert!(
            snap["origin"].is_string(),
            "snapshot must have origin: {snap}"
        );
        // name and hash can be null.
        assert!(snap["age"].is_string(), "snapshot must have age: {snap}");
        assert!(
            snap["files_count"].is_number(),
            "snapshot must have files_count: {snap}"
        );
        assert!(
            snap["changes"].is_array(),
            "snapshot must have changes array: {snap}"
        );
    }
}

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

// -----------------------------------------------------------------------
// AB-007: truncate_path -- char-boundary safe
// -----------------------------------------------------------------------

#[test]
fn truncate_path_ascii_under_max_returns_as_is() {
    assert_eq!(truncate_path("/a/b/c", 33), "/a/b/c");
}

#[test]
fn truncate_path_ascii_over_max_keeps_last_chars_with_ellipsis() {
    let path = "a".repeat(50);
    let out = truncate_path(&path, 33);
    assert_eq!(out.chars().count(), 33);
    assert!(out.starts_with("..."));
    assert_eq!(&out[3..], &"a".repeat(30));
}

#[test]
fn truncate_path_unicode_under_max_chars_is_kept_even_if_byte_len_exceeds() {
    // 10 CJK chars = 30 bytes; max 33 chars; should pass through unchanged.
    let path = "日".repeat(10);
    assert_eq!(truncate_path(&path, 33), path);
}

#[test]
fn truncate_path_unicode_does_not_panic_at_codepoint_boundary() {
    // AB-007 regression: with the legacy byte-slice implementation this
    // input panicked with "byte index N is not a char boundary" because
    // the suffix started in the middle of a multibyte character.
    //
    // 40 CJK (`日`, 3 bytes each) + 1 ASCII = 41 chars, 121 bytes.
    // max = 33. Legacy code computed slice start =
    // `path.len() - (max - 3) = 121 - 30 = 91`, which lands inside the
    // 31st `日` (bytes 90-92).
    let path = format!("{}a", "日".repeat(40));
    let out = truncate_path(&path, 33);
    assert!(out.starts_with("..."));
    assert_eq!(out.chars().count(), 33);
    let suffix: String = out.chars().skip(3).collect();
    assert_eq!(suffix, format!("{}a", "日".repeat(29)));
}

#[test]
fn truncate_path_unicode_over_max_uses_char_count_not_byte_count() {
    // 40 CJK chars = 120 bytes; max 33 chars; want last 30 chars + "...".
    let path = "日".repeat(40);
    let out = truncate_path(&path, 33);
    assert_eq!(out.chars().count(), 33);
    assert!(out.starts_with("..."));
    let suffix: String = out.chars().skip(3).collect();
    assert_eq!(suffix, "日".repeat(30));
}

#[test]
fn truncate_path_empty_string_returns_empty() {
    assert_eq!(truncate_path("", 33), "");
}

#[test]
fn truncate_path_max_three_returns_last_three_chars_no_ellipsis() {
    // With max == 3 there is no room for both an ellipsis and content;
    // returning the last `max` chars (no ellipsis) is more useful than
    // returning just "..." -- and importantly does not panic.
    let path = "abcdefghij";
    assert_eq!(truncate_path(path, 3), "hij");
}

#[test]
fn truncate_path_max_zero_does_not_panic() {
    // Defensive: ill-typed callers must not bring down snapshot rendering.
    let _ = truncate_path("abcdef", 0);
    let _ = truncate_path("日本語", 0);
}
