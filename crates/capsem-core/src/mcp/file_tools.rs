//! Built-in MCP tools for workspace snapshot tracking and revert.
//!
//! - `snapshots_changes`: diff current workspace against auto-snapshot checkpoints
//! - `snapshots_list`: list all snapshots with per-snapshot diffs
//! - `snapshots_revert`: restore a file from a checkpoint to the current workspace
//! - `snapshots_create`: create a named manual snapshot
//! - `snapshots_delete`: delete a manual snapshot
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
    "snapshots_changes", "snapshots_list", "snapshots_revert", "snapshots_create", "snapshots_delete", "snapshots_history",
];

pub fn is_file_tool(name: &str) -> bool {
    FILE_TOOL_NAMES.contains(&name)
}

/// Return tool definitions for file tools.
pub fn file_tool_defs() -> Vec<McpToolDef> {
    vec![
        McpToolDef {
            namespaced_name: "snapshots_changes".into(),
            original_name: "snapshots_changes".into(),
            description: Some(concat!(
                "List files that have changed in the workspace compared to automatic checkpoints. ",
                "Each entry includes the file path, operation (created/modified/deleted), size, ",
                "and a checkpoint ID that can be passed to snapshots_revert. ",
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
            namespaced_name: "snapshots_list".into(),
            original_name: "snapshots_list".into(),
            description: Some(concat!(
                "List all workspace snapshots (automatic and manual). ",
                "Returns slot index, origin (auto/manual), name, age, blake3 hash, ",
                "and a changes array showing created/modified/deleted files vs current workspace. ",
                "Empty snapshots (zero files) are filtered out.",
            ).into()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
            server_name: "builtin".into(),
            annotations: Some(ToolAnnotations {
                title: Some("List snapshots".into()),
                read_only_hint: true,
                destructive_hint: false,
                idempotent_hint: true,
                open_world_hint: false,
            }),
        },
        McpToolDef {
            namespaced_name: "snapshots_revert".into(),
            original_name: "snapshots_revert".into(),
            description: Some(concat!(
                "Revert a file to its state at a specific checkpoint. ",
                "Use the checkpoint ID from snapshots_changes output, or omit checkpoint ",
                "to auto-select the most recent snapshot containing the file. ",
                "If the file was created after the checkpoint, it is deleted. ",
                "If the file was modified, it is restored to its checkpoint state. ",
                "Changes are reflected immediately in the guest via VirtioFS.",
            ).into()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative file path from snapshots_changes output (e.g., 'project/app.js')"
                    },
                    "checkpoint": {
                        "type": "string",
                        "description": "Checkpoint ID (e.g., 'cp-0'). Optional: defaults to the most recent snapshot containing the file."
                    }
                },
                "required": ["path"]
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
            namespaced_name: "snapshots_create".into(),
            original_name: "snapshots_create".into(),
            description: Some(concat!(
                "Create a named workspace snapshot (checkpoint). ",
                "The snapshot captures the current state of all files and can be used ",
                "with snapshots_revert to restore files later. Returns the checkpoint ID, ",
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
            namespaced_name: "snapshots_delete".into(),
            original_name: "snapshots_delete".into(),
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
        McpToolDef {
            namespaced_name: "snapshots_history".into(),
            original_name: "snapshots_history".into(),
            description: Some(concat!(
                "Show the history of a specific file across all snapshots. ",
                "For each snapshot that contains a version of the file, shows the checkpoint, ",
                "origin, age, size, and whether the file was created, modified, or unchanged. ",
                "Accepts both relative paths (hello.txt) and absolute guest paths (/root/hello.txt).",
            ).into()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path (e.g., 'hello.txt' or '/root/hello.txt')"
                    }
                },
                "required": ["path"]
            }),
            server_name: "builtin".into(),
            annotations: Some(ToolAnnotations {
                title: Some("File history".into()),
                read_only_hint: true,
                destructive_hint: false,
                idempotent_hint: true,
                open_world_hint: false,
            }),
        },
    ]
}

/// Normalize and validate a path. Strips `/root/` prefix (guest workspace mount)
/// so both `hello.txt` and `/root/hello.txt` work. Rejects traversal and null bytes.
fn normalize_path(path: &str) -> Result<String, String> {
    let path = path.strip_prefix("/root/").unwrap_or(path);
    if path.is_empty() {
        return Err("path is empty".into());
    }
    if path.starts_with('/') {
        return Err("absolute paths not allowed (use relative or /root/ prefix)".into());
    }
    if path.contains("..") {
        return Err("path traversal not allowed".into());
    }
    if path.contains('\0') {
        return Err("null bytes not allowed in path".into());
    }
    Ok(path.to_string())
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

/// Compute per-snapshot change entries: created/modified/deleted vs current workspace.
fn compute_snapshot_changes(
    snap_files: &HashMap<String, u64>,
    current_files: &HashMap<String, u64>,
) -> Vec<Value> {
    let mut changes = Vec::new();

    // Created: in current but not in snapshot (file was created after snapshot).
    for (path, size) in current_files {
        if !snap_files.contains_key(path) {
            changes.push(serde_json::json!({
                "path": path,
                "op": "created",
                "size": size,
            }));
        }
    }

    // Deleted: in snapshot but not in current (file was deleted after snapshot).
    for path in snap_files.keys() {
        if !current_files.contains_key(path) {
            changes.push(serde_json::json!({
                "path": path,
                "op": "deleted",
            }));
        }
    }

    // Modified: in both but different size.
    for (path, current_size) in current_files {
        if let Some(snap_size) = snap_files.get(path) {
            if current_size != snap_size {
                changes.push(serde_json::json!({
                    "path": path,
                    "op": "modified",
                    "size": current_size,
                }));
            }
        }
    }

    // Sort by path for deterministic output.
    changes.sort_by(|a, b| {
        let pa = a["path"].as_str().unwrap_or("");
        let pb = b["path"].as_str().unwrap_or("");
        pa.cmp(pb)
    });

    changes
}

/// Handle `snapshots_changes` tool call.
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

/// Handle `snapshots_revert` tool call.
pub fn handle_revert_file(
    arguments: &Value,
    scheduler: &AutoSnapshotScheduler,
    workspace_root: &Path,
    request_id: Option<Value>,
) -> JsonRpcResponse {
    let raw_path = match arguments.get("path").and_then(|v| v.as_str()) {
        Some(p) => p,
        None => return JsonRpcResponse::err(request_id, -32602, "missing 'path' argument"),
    };

    // Normalize and validate path (strips /root/ prefix if present).
    let path_str = match normalize_path(raw_path) {
        Ok(p) => p,
        Err(e) => return JsonRpcResponse::err(request_id, -32602, format!("invalid path: {e}")),
    };

    // Resolve checkpoint: explicit or auto-select newest containing the file.
    let (slot, cp_str_owned) = if let Some(cp_str) = arguments.get("checkpoint").and_then(|v| v.as_str()) {
        let slot = match parse_checkpoint(cp_str) {
            Ok(s) => s,
            Err(e) => return JsonRpcResponse::err(request_id, -32602, e),
        };
        (slot, cp_str.to_string())
    } else {
        // Auto-select: scan snapshots newest-first, find first containing the file.
        let snapshots = scheduler.list_snapshots();
        let found = snapshots.iter().find(|s| {
            s.workspace_path.join(&path_str).exists()
        });
        match found {
            Some(s) => (s.slot, format!("cp-{}", s.slot)),
            None => {
                return JsonRpcResponse::err(
                    request_id,
                    -32602,
                    "no snapshot contains this file",
                );
            }
        }
    };

    // Get snapshot.
    let snap = match scheduler.get_snapshot(slot) {
        Some(s) => s,
        None => {
            return JsonRpcResponse::err(
                request_id,
                -32602,
                format!("checkpoint {} not found", cp_str_owned),
            )
        }
    };

    let snap_file = snap.workspace_path.clone().join(&path_str);
    let current_file = workspace_root.join(&path_str);

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

    let action;
    // Check if file already matches snapshot (no-op): same content AND same permissions.
    if snap_file.exists() && current_file.exists() {
        if let (Ok(snap_bytes), Ok(cur_bytes)) = (
            std::fs::read(&snap_file),
            std::fs::read(&current_file),
        ) {
            let same_perms = match (snap_file.metadata(), current_file.metadata()) {
                (Ok(sm), Ok(cm)) => {
                    use std::os::unix::fs::PermissionsExt;
                    sm.permissions().mode() == cm.permissions().mode()
                }
                _ => true, // can't read metadata, assume same
            };
            if snap_bytes == cur_bytes && same_perms {
                return JsonRpcResponse::err(
                    request_id,
                    -32602,
                    "file already matches snapshot (already current)",
                );
            }
        }
    } else if !snap_file.exists() && !current_file.exists() {
        return JsonRpcResponse::err(
            request_id,
            -32602,
            "file does not exist in snapshot or workspace",
        );
    }

    if snap_file.exists() {
        // File exists in snapshot -- restore it.
        action = "restored";
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
        // Restore permissions from snapshot.
        if let Ok(snap_meta) = snap_file.metadata() {
            let _ = std::fs::set_permissions(&current_file, snap_meta.permissions());
        }
    } else {
        // File was created after checkpoint -- delete it.
        action = "deleted";
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
                "action": action,
                "checkpoint": cp_str_owned,
            }).to_string()}]
        }),
    )
}

/// Handle `snapshots_list` tool call -- return all snapshot metadata with per-snapshot diffs.
///
/// Changes are computed vs the PREVIOUS snapshot (oldest-first), not vs current workspace.
/// This shows what changed AT the time of each snapshot.
pub fn handle_list_snapshots(
    scheduler: &AutoSnapshotScheduler,
    workspace_root: &Path,
    request_id: Option<Value>,
) -> JsonRpcResponse {
    let mut snapshots = scheduler.list_snapshots();
    // list_snapshots returns newest-first; reverse to walk oldest-first.
    snapshots.reverse();

    let mut prev_files: HashMap<String, u64> = HashMap::new();
    let mut entries: Vec<serde_json::Value> = Vec::new();

    for s in &snapshots {
        let snap_files = collect_files(&s.workspace_path);
        let origin = match s.origin {
            SnapshotOrigin::Auto => "auto",
            SnapshotOrigin::Manual => "manual",
        };

        // Diff this snapshot vs previous snapshot (not vs current workspace).
        // "new" = in this snap but not in prev, "deleted" = in prev but not here,
        // "modified" = in both but different size.
        let changes = compute_changes_vs_previous(&snap_files, &prev_files);

        entries.push(serde_json::json!({
            "checkpoint": format!("cp-{}", s.slot),
            "slot": s.slot,
            "origin": origin,
            "name": s.name,
            "hash": s.hash,
            "age": age_string(s.timestamp),
            "files_count": snap_files.len(),
            "changes": changes,
        }));

        prev_files = snap_files;
    }

    // Return newest-first (reverse back).
    entries.reverse();

    let summary = serde_json::json!({
        "snapshots": entries,
        "auto_max": scheduler.max_auto(),
        "manual_max": scheduler.max_manual(),
        "manual_available": scheduler.available_manual_slots(),
    });
    JsonRpcResponse::ok(
        request_id,
        serde_json::json!({
            "content": [{"type": "text", "text": summary.to_string()}]
        }),
    )
}

/// Compute changes between two snapshots: what's new/modified/deleted in `current` vs `prev`.
fn compute_changes_vs_previous(
    current: &HashMap<String, u64>,
    prev: &HashMap<String, u64>,
) -> Vec<Value> {
    let mut changes = Vec::new();

    // New: in current but not in prev.
    for (path, size) in current {
        if !prev.contains_key(path) {
            changes.push(serde_json::json!({"path": path, "op": "new", "size": size}));
        }
    }

    // Deleted: in prev but not in current.
    for path in prev.keys() {
        if !current.contains_key(path) {
            changes.push(serde_json::json!({"path": path, "op": "deleted"}));
        }
    }

    // Modified: in both but different size.
    for (path, cur_size) in current {
        if let Some(prev_size) = prev.get(path) {
            if cur_size != prev_size {
                changes.push(serde_json::json!({"path": path, "op": "modified", "size": cur_size}));
            }
        }
    }

    changes.sort_by(|a, b| {
        let pa = a["path"].as_str().unwrap_or("");
        let pb = b["path"].as_str().unwrap_or("");
        pa.cmp(pb)
    });
    changes
}

/// Handle `snapshots_create` tool call -- create a named manual snapshot.
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

/// Handle `snapshots_delete` tool call -- delete a manual snapshot.
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

/// Handle `snapshots_history` tool call -- show all versions of a file across snapshots.
pub fn handle_snapshots_history(
    arguments: &Value,
    scheduler: &AutoSnapshotScheduler,
    workspace_root: &Path,
    request_id: Option<Value>,
) -> JsonRpcResponse {
    let raw_path = match arguments.get("path").and_then(|v| v.as_str()) {
        Some(p) => p,
        None => return JsonRpcResponse::err(request_id, -32602, "missing 'path' argument"),
    };

    let path_str = match normalize_path(raw_path) {
        Ok(p) => p,
        Err(e) => return JsonRpcResponse::err(request_id, -32602, format!("invalid path: {e}")),
    };

    let mut snapshots = scheduler.list_snapshots();
    // Walk oldest-first to compute sequential status.
    snapshots.reverse();

    let current_file = workspace_root.join(&path_str);
    let current_size = current_file.metadata().ok().map(|m| m.len());

    let mut versions: Vec<serde_json::Value> = Vec::new();
    let mut prev_size: Option<u64> = None; // None = file didn't exist in previous snap

    for snap in &snapshots {
        let snap_file = snap.workspace_path.join(&path_str);
        let snap_size = snap_file.metadata().ok().map(|m| m.len());

        // Compare this version to PREVIOUS snapshot version.
        let status = match (snap_size, prev_size) {
            (Some(ss), Some(ps)) if ss == ps => "unchanged",
            (Some(_), Some(_)) => "modified",
            (Some(_), None) => "new",
            (None, Some(_)) => "deleted",
            (None, None) => {
                // File not in this snapshot and not in previous -- skip.
                prev_size = snap_size;
                continue;
            }
        };

        let origin = match snap.origin {
            SnapshotOrigin::Auto => "auto",
            SnapshotOrigin::Manual => "manual",
        };

        versions.push(serde_json::json!({
            "checkpoint": format!("cp-{}", snap.slot),
            "origin": origin,
            "name": snap.name,
            "age": age_string(snap.timestamp),
            "size": snap_size,
            "status": status,
        }));

        prev_size = snap_size;
    }

    // Return newest-first.
    versions.reverse();

    let result = serde_json::json!({
        "path": path_str,
        "current_size": current_size,
        "versions": versions,
    });

    JsonRpcResponse::ok(
        request_id,
        serde_json::json!({
            "content": [{"type": "text", "text": result.to_string()}]
        }),
    )
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

        // Verify action and checkpoint in response.
        let result_text = resp.result.unwrap()["content"][0]["text"].as_str().unwrap().to_string();
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
        let result_text = resp.result.unwrap()["content"][0]["text"].as_str().unwrap().to_string();
        let result: Value = serde_json::from_str(&result_text).unwrap();
        assert_eq!(result["action"], "restored");
        assert_eq!(result["checkpoint"], "cp-0");

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

        let resp = handle_list_snapshots(&sched, &ws, Some(serde_json::json!(1)));
        let text = resp.result.unwrap()["content"][0]["text"].as_str().unwrap().to_string();
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

        let resp = handle_list_snapshots(&sched, &ws, Some(serde_json::json!(1)));
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
        let resp = handle_revert_file(&args, &sched, &ws, Some(serde_json::json!(1)));
        assert!(resp.error.is_none(), "auto-select revert failed: {:?}", resp.error);

        let result_text = resp.result.unwrap()["content"][0]["text"].as_str().unwrap().to_string();
        let result: Value = serde_json::from_str(&result_text).unwrap();
        assert_eq!(result["action"], "restored");
        assert_eq!(result["checkpoint"], "cp-1");
        assert_eq!(std::fs::read_to_string(ws.join("auto.txt")).unwrap(), "version 2 is longer");
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
        let resp = handle_revert_file(&args, &sched, &ws, Some(serde_json::json!(1)));
        assert!(resp.error.is_some());
        let err_msg = &resp.error.unwrap().message;
        assert!(err_msg.contains("no snapshot contains this file"), "unexpected error: {err_msg}");
    }
}
