//! Built-in MCP tools for workspace file tracking and revert.
//!
//! - `list_changed_files`: diff current workspace against auto-snapshot checkpoints
//! - `revert_file`: restore a file from a checkpoint to the current workspace
//!
//! These tools operate entirely on the host filesystem (VirtioFS directories).
//! The guest sees changes immediately via VirtioFS.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;
use walkdir::WalkDir;

use crate::auto_snapshot::{AutoSnapshotScheduler, SnapshotOrigin};

use super::types::{JsonRpcResponse, McpToolDef, ToolAnnotations};

/// Tool names for file operations.
pub const FILE_TOOL_NAMES: &[&str] = &[
    "list_changed_files", "revert_file", "snapshot", "delete_snapshot",
];

pub fn is_file_tool(name: &str) -> bool {
    FILE_TOOL_NAMES.contains(&name)
}

/// Return tool definitions for file tools.
pub fn file_tool_defs() -> Vec<McpToolDef> {
    vec![
        McpToolDef {
            namespaced_name: "list_changed_files".into(),
            original_name: "list_changed_files".into(),
            description: Some(concat!(
                "List files that have changed in the workspace compared to automatic checkpoints. ",
                "Each entry includes the file path, operation (created/modified/deleted), size, ",
                "and a checkpoint ID that can be passed to revert_file. ",
                "Shows newest changes first.",
            ).into()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
            server_name: "builtin".into(),
            annotations: Some(ToolAnnotations {
                title: Some("List changed files".into()),
                read_only_hint: true,
                destructive_hint: false,
                idempotent_hint: true,
                open_world_hint: false,
            }),
        },
        McpToolDef {
            namespaced_name: "revert_file".into(),
            original_name: "revert_file".into(),
            description: Some(concat!(
                "Revert a file to its state at a specific checkpoint. ",
                "Use the checkpoint ID from list_changed_files output. ",
                "If the file was created after the checkpoint, it is deleted. ",
                "If the file was modified, it is restored to its checkpoint state. ",
                "Changes are reflected immediately in the guest via VirtioFS.",
            ).into()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative file path from list_changed_files output (e.g., 'project/app.js')"
                    },
                    "checkpoint": {
                        "type": "string",
                        "description": "Checkpoint ID from list_changed_files output (e.g., 'cp-0')"
                    }
                },
                "required": ["path", "checkpoint"]
            }),
            server_name: "builtin".into(),
            annotations: Some(ToolAnnotations {
                title: Some("Revert file".into()),
                read_only_hint: false,
                destructive_hint: true,
                idempotent_hint: true,
                open_world_hint: false,
            }),
        },
        McpToolDef {
            namespaced_name: "snapshot".into(),
            original_name: "snapshot".into(),
            description: Some(concat!(
                "Create a named workspace snapshot (checkpoint). ",
                "The snapshot captures the current state of all files and can be used ",
                "with revert_file to restore files later. Returns the checkpoint ID, ",
                "a blake3 hash of the workspace, and the number of remaining snapshot slots.",
            ).into()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Label for this snapshot (alphanumeric, underscore, hyphen; max 64 chars)"
                    }
                },
                "required": ["name"]
            }),
            server_name: "builtin".into(),
            annotations: Some(ToolAnnotations {
                title: Some("Create snapshot".into()),
                read_only_hint: false,
                destructive_hint: false,
                idempotent_hint: false,
                open_world_hint: false,
            }),
        },
        McpToolDef {
            namespaced_name: "delete_snapshot".into(),
            original_name: "delete_snapshot".into(),
            description: Some(concat!(
                "Delete a manual snapshot by checkpoint ID. ",
                "Only manual (named) snapshots can be deleted. ",
                "Automatic snapshots are managed by the scheduler.",
            ).into()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "checkpoint": {
                        "type": "string",
                        "description": "Checkpoint ID to delete (e.g., 'cp-12')"
                    }
                },
                "required": ["checkpoint"]
            }),
            server_name: "builtin".into(),
            annotations: Some(ToolAnnotations {
                title: Some("Delete snapshot".into()),
                read_only_hint: false,
                destructive_hint: true,
                idempotent_hint: true,
                open_world_hint: false,
            }),
        },
    ]
}

/// Validate a relative path: no `..`, no absolute, no null bytes.
fn validate_path(path: &str) -> Result<&str, String> {
    if path.is_empty() {
        return Err("path is empty".into());
    }
    if path.starts_with('/') {
        return Err("absolute paths not allowed".into());
    }
    if path.contains("..") {
        return Err("path traversal not allowed".into());
    }
    if path.contains('\0') {
        return Err("null bytes not allowed in path".into());
    }
    Ok(path)
}

/// Parse checkpoint ID like "cp-3" into slot index 3.
fn parse_checkpoint(cp: &str) -> Result<usize, String> {
    cp.strip_prefix("cp-")
        .and_then(|s| s.parse::<usize>().ok())
        .ok_or_else(|| format!("invalid checkpoint ID: {cp:?}"))
}

/// Validate a snapshot name: alphanumeric + underscore + hyphen, 1-64 chars.
fn validate_snapshot_name(name: &str) -> Result<&str, String> {
    if name.is_empty() || name.len() > 64 {
        return Err("name must be 1-64 characters".into());
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err("name must be alphanumeric, underscore, or hyphen only".into());
    }
    Ok(name)
}

/// Entry describing a changed file.
#[derive(Debug, serde::Serialize)]
struct ChangedFile {
    path: String,
    op: &'static str,
    size: Option<u64>,
    checkpoint: String,
    checkpoint_age: String,
    checkpoint_origin: String,
    checkpoint_name: Option<String>,
}

/// Collect file listing from a directory (relative paths + sizes).
fn collect_files(root: &Path) -> HashMap<String, u64> {
    let mut files = HashMap::new();
    if !root.exists() {
        return files;
    }
    for entry in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        if let Ok(rel) = entry.path().strip_prefix(root) {
            let rel_str = rel.to_string_lossy().to_string();
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            files.insert(rel_str, size);
        }
    }
    files
}

fn age_string(ts: SystemTime) -> String {
    let elapsed = ts.elapsed().unwrap_or_default();
    let mins = elapsed.as_secs() / 60;
    if mins == 0 {
        "just now".to_string()
    } else if mins == 1 {
        "1 min ago".to_string()
    } else if mins < 60 {
        format!("{mins} min ago")
    } else {
        let hours = mins / 60;
        format!("{hours} hr ago")
    }
}

/// Handle `list_changed_files` tool call.
pub fn handle_list_changed_files(
    scheduler: &AutoSnapshotScheduler,
    workspace_root: &Path,
    request_id: Option<Value>,
) -> JsonRpcResponse {
    let current_files = collect_files(workspace_root);
    let snapshots = scheduler.list_snapshots();

    if snapshots.is_empty() {
        return JsonRpcResponse::ok(
            request_id,
            serde_json::json!({
                "content": [{"type": "text", "text": "No checkpoints available yet."}]
            }),
        );
    }

    let mut changes: Vec<ChangedFile> = Vec::new();
    let mut seen_paths: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Walk snapshots newest-first. For each, diff against current.
    // Only report each path once (from the most recent checkpoint that shows the change).
    for snap in &snapshots {
        let snap_root = snap.workspace_path.clone();
        let snap_files = collect_files(&snap_root);
        let cp_id = format!("cp-{}", snap.slot);
        let age = age_string(snap.timestamp);
        let origin_str = match snap.origin {
            SnapshotOrigin::Auto => "auto",
            SnapshotOrigin::Manual => "manual",
        };

        // Created: in current but not in snapshot.
        for (path, size) in &current_files {
            if !snap_files.contains_key(path) && seen_paths.insert(path.clone()) {
                changes.push(ChangedFile {
                    path: path.clone(),
                    op: "created",
                    size: Some(*size),
                    checkpoint: cp_id.clone(),
                    checkpoint_age: age.clone(),
                    checkpoint_origin: origin_str.into(),
                    checkpoint_name: snap.name.clone(),
                });
            }
        }

        // Deleted: in snapshot but not in current.
        for path in snap_files.keys() {
            if !current_files.contains_key(path) && seen_paths.insert(path.clone()) {
                changes.push(ChangedFile {
                    path: path.clone(),
                    op: "deleted",
                    size: None,
                    checkpoint: cp_id.clone(),
                    checkpoint_age: age.clone(),
                    checkpoint_origin: origin_str.into(),
                    checkpoint_name: snap.name.clone(),
                });
            }
        }

        // Modified: in both but different size.
        for (path, current_size) in &current_files {
            if let Some(snap_size) = snap_files.get(path) {
                if current_size != snap_size && seen_paths.insert(path.clone()) {
                    changes.push(ChangedFile {
                        path: path.clone(),
                        op: "modified",
                        size: Some(*current_size),
                        checkpoint: cp_id.clone(),
                        checkpoint_age: age.clone(),
                        checkpoint_origin: origin_str.into(),
                        checkpoint_name: snap.name.clone(),
                    });
                }
            }
        }
    }

    let text = serde_json::to_string_pretty(&changes).unwrap_or_else(|_| "[]".into());
    JsonRpcResponse::ok(
        request_id,
        serde_json::json!({
            "content": [{"type": "text", "text": text}]
        }),
    )
}

/// Handle `revert_file` tool call.
pub fn handle_revert_file(
    arguments: &Value,
    scheduler: &AutoSnapshotScheduler,
    workspace_root: &Path,
    request_id: Option<Value>,
) -> JsonRpcResponse {
    let path_str = match arguments.get("path").and_then(|v| v.as_str()) {
        Some(p) => p,
        None => return JsonRpcResponse::err(request_id, -32602, "missing 'path' argument"),
    };
    let cp_str = match arguments.get("checkpoint").and_then(|v| v.as_str()) {
        Some(c) => c,
        None => return JsonRpcResponse::err(request_id, -32602, "missing 'checkpoint' argument"),
    };

    // Validate path.
    if let Err(e) = validate_path(path_str) {
        return JsonRpcResponse::err(request_id, -32602, format!("invalid path: {e}"));
    }

    // Parse checkpoint.
    let slot = match parse_checkpoint(cp_str) {
        Ok(s) => s,
        Err(e) => return JsonRpcResponse::err(request_id, -32602, e),
    };

    // Get snapshot.
    let snap = match scheduler.get_snapshot(slot) {
        Some(s) => s,
        None => {
            return JsonRpcResponse::err(
                request_id,
                -32602,
                format!("checkpoint {cp_str} not found"),
            )
        }
    };

    let snap_file = snap.workspace_path.clone().join(path_str);
    let current_file = workspace_root.join(path_str);

    // Check for symlink escape: canonicalize both paths to handle macOS /var -> /private/var.
    if let (Ok(resolved_file), Ok(resolved_root)) =
        (current_file.canonicalize(), workspace_root.canonicalize())
    {
        if !resolved_file.starts_with(&resolved_root) {
            return JsonRpcResponse::err(
                request_id,
                -32602,
                "path resolves outside workspace (symlink escape)",
            );
        }
    }

    if snap_file.exists() {
        // File exists in snapshot -- restore it.
        if let Some(parent) = current_file.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return JsonRpcResponse::err(
                    request_id,
                    -32603,
                    format!("failed to create parent directory: {e}"),
                );
            }
        }
        if let Err(e) = std::fs::copy(&snap_file, &current_file) {
            return JsonRpcResponse::err(
                request_id,
                -32603,
                format!("failed to restore file: {e}"),
            );
        }
    } else {
        // File was created after checkpoint -- delete it.
        if current_file.exists() {
            if let Err(e) = std::fs::remove_file(&current_file) {
                return JsonRpcResponse::err(
                    request_id,
                    -32603,
                    format!("failed to delete file: {e}"),
                );
            }
        }
    }

    JsonRpcResponse::ok(
        request_id,
        serde_json::json!({
            "content": [{"type": "text", "text": serde_json::json!({
                "reverted": true,
                "path": path_str,
            }).to_string()}]
        }),
    )
}

/// Handle `snapshot` tool call — create a named manual snapshot.
pub fn handle_snapshot(
    arguments: &Value,
    scheduler: &mut AutoSnapshotScheduler,
    request_id: Option<Value>,
) -> JsonRpcResponse {
    let name = match arguments.get("name").and_then(|v| v.as_str()) {
        Some(n) => n,
        None => return JsonRpcResponse::err(request_id, -32602, "missing 'name' argument"),
    };
    if let Err(e) = validate_snapshot_name(name) {
        return JsonRpcResponse::err(request_id, -32602, format!("invalid name: {e}"));
    }

    match scheduler.take_named_snapshot(name) {
        Ok(slot) => {
            let available = scheduler.available_manual_slots();
            JsonRpcResponse::ok(
                request_id,
                serde_json::json!({
                    "content": [{"type": "text", "text": serde_json::json!({
                        "checkpoint": format!("cp-{}", slot.slot),
                        "name": name,
                        "hash": slot.hash,
                        "available": available,
                    }).to_string()}]
                }),
            )
        }
        Err(e) => JsonRpcResponse::err(request_id, -32603, format!("{e}")),
    }
}

/// Handle `delete_snapshot` tool call — delete a manual snapshot.
pub fn handle_delete_snapshot(
    arguments: &Value,
    scheduler: &AutoSnapshotScheduler,
    request_id: Option<Value>,
) -> JsonRpcResponse {
    let cp_str = match arguments.get("checkpoint").and_then(|v| v.as_str()) {
        Some(c) => c,
        None => return JsonRpcResponse::err(request_id, -32602, "missing 'checkpoint' argument"),
    };
    let slot = match parse_checkpoint(cp_str) {
        Ok(s) => s,
        Err(e) => return JsonRpcResponse::err(request_id, -32602, e),
    };

    // Only allow deleting manual snapshots.
    match scheduler.get_metadata(slot) {
        Some(meta) if meta.origin == SnapshotOrigin::Auto => {
            return JsonRpcResponse::err(
                request_id, -32602,
                "cannot delete automatic snapshots (managed by scheduler)",
            );
        }
        None => {
            return JsonRpcResponse::err(
                request_id, -32602,
                format!("checkpoint {cp_str} not found"),
            );
        }
        _ => {}
    }

    match scheduler.delete_snapshot(slot) {
        Ok(()) => JsonRpcResponse::ok(
            request_id,
            serde_json::json!({
                "content": [{"type": "text", "text": serde_json::json!({
                    "deleted": true,
                    "checkpoint": cp_str,
                }).to_string()}]
            }),
        ),
        Err(e) => JsonRpcResponse::err(request_id, -32603, format!("{e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auto_snapshot::AutoSnapshotScheduler;
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
    fn validate_path_rejects_traversal() {
        assert!(validate_path("../etc/passwd").is_err());
        assert!(validate_path("foo/../../bar").is_err());
    }

    #[test]
    fn validate_path_rejects_absolute() {
        assert!(validate_path("/etc/passwd").is_err());
    }

    #[test]
    fn validate_path_rejects_empty() {
        assert!(validate_path("").is_err());
    }

    #[test]
    fn validate_path_rejects_null_bytes() {
        assert!(validate_path("foo\0bar").is_err());
    }

    #[test]
    fn validate_path_accepts_normal() {
        assert!(validate_path("project/app.js").is_ok());
        assert!(validate_path("a.txt").is_ok());
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
        let resp = handle_list_changed_files(&sched, &workspace, Some(serde_json::json!(1)));
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
        let resp = handle_list_changed_files(&sched, &workspace, Some(serde_json::json!(1)));
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
        let resp = handle_list_changed_files(&sched, &workspace, Some(serde_json::json!(1)));
        let text = resp.result.unwrap()["content"][0]["text"].as_str().unwrap().to_string();
        let changes: Vec<Value> = serde_json::from_str(&text).unwrap();

        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0]["path"], "gone.txt");
        assert_eq!(changes[0]["op"], "deleted");
    }

    /// Roundtrip test: write a file, snapshot, copy it, delete original,
    /// revert via revert_file, verify content matches exactly.
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

        // Revert via revert_file.
        let args = serde_json::json!({"path": "important.txt", "checkpoint": "cp-0"});
        let resp = handle_revert_file(
            &args,
            &sched,
            &session.join("workspace"),
            Some(serde_json::json!(1)),
        );

        // Verify success.
        assert!(resp.error.is_none(), "revert_file failed: {:?}", resp.error);
        let result_text = resp.result.unwrap()["content"][0]["text"].as_str().unwrap().to_string();
        assert!(result_text.contains("\"reverted\":true") || result_text.contains("\"reverted\": true"));

        // Verify the file is back with exact same content.
        let recovered = std::fs::read_to_string(session.join("workspace/important.txt")).unwrap();
        assert_eq!(recovered, content, "recovered content must match original exactly");
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
        );

        assert!(resp.error.is_none());
        assert!(!session.join("workspace/new.txt").exists());
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
        let resp = handle_revert_file(&args, &sched, &ws, Some(serde_json::json!(1)));
        assert!(resp.error.is_none(), "revert to cp-0 failed: {:?}", resp.error);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "version ONE");

        // Revert to version 2 (cp-1)
        let args = serde_json::json!({"path": "evolving.txt", "checkpoint": "cp-1"});
        let resp = handle_revert_file(&args, &sched, &ws, Some(serde_json::json!(1)));
        assert!(resp.error.is_none(), "revert to cp-1 failed: {:?}", resp.error);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "version TWO -- longer content here");

        // Revert to version 3 (cp-2)
        let args = serde_json::json!({"path": "evolving.txt", "checkpoint": "cp-2"});
        let resp = handle_revert_file(&args, &sched, &ws, Some(serde_json::json!(1)));
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
        let resp = handle_revert_file(&args, &sched, &ws, Some(serde_json::json!(1)));
        assert!(resp.error.is_none());

        // Verify exact content
        let recovered = std::fs::read_to_string(&file).unwrap();
        assert_eq!(recovered, content, "recovered content must match original exactly");
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

        let resp = handle_list_changed_files(&sched, &ws, Some(serde_json::json!(1)));
        let text = resp.result.unwrap()["content"][0]["text"].as_str().unwrap().to_string();
        let changes: Vec<Value> = serde_json::from_str(&text).unwrap();

        // Should see: brand_new.txt (created), modify_me.txt (modified), delete_me.txt (deleted)
        // keep.txt should NOT appear (unchanged).
        let paths: Vec<&str> = changes.iter().map(|c| c["path"].as_str().unwrap()).collect();
        assert!(paths.contains(&"brand_new.txt"), "missing created file: {paths:?}");
        assert!(paths.contains(&"modify_me.txt"), "missing modified file: {paths:?}");
        assert!(paths.contains(&"delete_me.txt"), "missing deleted file: {paths:?}");
        assert!(!paths.contains(&"keep.txt"), "unchanged file should not appear: {paths:?}");

        // Verify ops
        let get_op = |name: &str| -> &str {
            changes.iter().find(|c| c["path"] == name).unwrap()["op"].as_str().unwrap()
        };
        assert_eq!(get_op("brand_new.txt"), "created");
        assert_eq!(get_op("modify_me.txt"), "modified");
        assert_eq!(get_op("delete_me.txt"), "deleted");
    }
}
