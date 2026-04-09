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
use std::path::Path;
use std::sync::Arc;
use std::time::SystemTime;

use serde_json::Value;
use walkdir::WalkDir;

use crate::auto_snapshot::{AutoSnapshotScheduler, SnapshotOrigin};

use super::builtin_tools::{paginate, DEFAULT_MAX_LENGTH};
use super::types::{JsonRpcResponse, McpToolDef, ToolAnnotations};

/// Tool names for file operations.
pub const FILE_TOOL_NAMES: &[&str] = &[
    "snapshots_changes", "snapshots_list", "snapshots_revert", "snapshots_create", "snapshots_delete", "snapshots_history", "snapshots_compact",
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
                "Shows newest changes first. Output is paginated (default 5000 chars).",
            ).into()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "start_index": {
                        "type": "integer",
                        "description": "Character offset to start from (default: 0). Use the value from the pagination hint to continue."
                    },
                    "max_length": {
                        "type": "integer",
                        "description": "Maximum characters to return (default: 5000). If truncated, a pagination hint shows the next start_index."
                    },
                    "format": {
                        "type": "string",
                        "enum": ["text", "json"],
                        "description": "Output format: 'text' (default) for a compact table, 'json' for machine-readable JSON array."
                    }
                }
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
                "Shows slot index, origin (auto/manual), name, age, blake3 hash, file count, ",
                "and a compact change summary. Output is paginated (default 5000 chars).",
            ).into()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "start_index": {
                        "type": "integer",
                        "description": "Character offset to start from (default: 0). Use the value from the pagination hint to continue."
                    },
                    "max_length": {
                        "type": "integer",
                        "description": "Maximum characters to return (default: 5000). If truncated, a pagination hint shows the next start_index."
                    },
                    "format": {
                        "type": "string",
                        "enum": ["text", "json"],
                        "description": "Output format: 'text' (default) for a compact table, 'json' for machine-readable JSON."
                    }
                }
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
        McpToolDef {
            namespaced_name: "snapshots_compact".into(),
            original_name: "snapshots_compact".into(),
            description: Some(concat!(
                "Compact multiple snapshots into a single new manual snapshot. ",
                "Merges workspaces with newest-file-wins strategy. ",
                "Deletes all source snapshots after successful compaction. ",
                "Frees snapshot slots while preserving file state.",
            ).into()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "checkpoints": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Checkpoint IDs to compact (e.g., ['cp-0', 'cp-1', 'cp-10'])"
                    },
                    "name": {
                        "type": "string",
                        "description": "Name for the compacted snapshot (optional, defaults to timestamp)"
                    }
                },
                "required": ["checkpoints"]
            }),
            server_name: "builtin".into(),
            annotations: Some(ToolAnnotations {
                title: Some("Compact snapshots".into()),
                read_only_hint: false,
                destructive_hint: true,
                idempotent_hint: false,
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

/// Entry from a directory walk: size and whether the entry is a symlink.
#[derive(Debug, Clone, Copy)]
struct FileEntry {
    size: u64,
    is_symlink: bool,
}

/// Entry describing a changed file.
#[derive(Debug, serde::Serialize)]
struct ChangedFile {
    path: String,
    op: &'static str,
    size: Option<u64>,
    is_symlink: bool,
    checkpoint: String,
    checkpoint_age: String,
    checkpoint_origin: String,
    checkpoint_name: Option<String>,
}

/// Collect file listing from a directory (relative paths + metadata).
/// Includes both regular files and symlinks. Does not follow symlinks.
fn collect_files(root: &Path) -> HashMap<String, FileEntry> {
    let mut files = HashMap::new();
    if !root.exists() {
        return files;
    }
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let ft = entry.file_type();
        if !ft.is_file() && !ft.is_symlink() {
            continue;
        }
        if let Ok(rel) = entry.path().strip_prefix(root) {
            let rel_str = rel.to_string_lossy().to_string();
            // Use symlink_metadata so we don't follow symlinks for size.
            let size = entry
                .path()
                .symlink_metadata()
                .map(|m| m.len())
                .unwrap_or(0);
            files.insert(rel_str, FileEntry { size, is_symlink: ft.is_symlink() });
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

/// Collect changes between current workspace and snapshots.
fn collect_changes(
    scheduler: &AutoSnapshotScheduler,
    workspace_root: &Path,
) -> Vec<ChangedFile> {
    let current_files = collect_files(workspace_root);
    let snapshots = scheduler.list_snapshots();
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
        for (path, entry) in &current_files {
            if !snap_files.contains_key(path) && seen_paths.insert(path.clone()) {
                changes.push(ChangedFile {
                    path: path.clone(),
                    op: "created",
                    size: Some(entry.size),
                    is_symlink: entry.is_symlink,
                    checkpoint: cp_id.clone(),
                    checkpoint_age: age.clone(),
                    checkpoint_origin: origin_str.into(),
                    checkpoint_name: snap.name.clone(),
                });
            }
        }

        // Deleted: in snapshot but not in current.
        for (path, entry) in &snap_files {
            if !current_files.contains_key(path) && seen_paths.insert(path.clone()) {
                changes.push(ChangedFile {
                    path: path.clone(),
                    op: "deleted",
                    size: None,
                    is_symlink: entry.is_symlink,
                    checkpoint: cp_id.clone(),
                    checkpoint_age: age.clone(),
                    checkpoint_origin: origin_str.into(),
                    checkpoint_name: snap.name.clone(),
                });
            }
        }

        // Modified: in both but different size.
        for (path, cur_entry) in &current_files {
            if let Some(snap_entry) = snap_files.get(path) {
                if cur_entry.size != snap_entry.size && seen_paths.insert(path.clone()) {
                    changes.push(ChangedFile {
                        path: path.clone(),
                        op: "modified",
                        size: Some(cur_entry.size),
                        is_symlink: cur_entry.is_symlink,
                        checkpoint: cp_id.clone(),
                        checkpoint_age: age.clone(),
                        checkpoint_origin: origin_str.into(),
                        checkpoint_name: snap.name.clone(),
                    });
                }
            }
        }
    }
    changes
}

/// Format bytes as human-readable size.
fn human_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1_048_576 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    }
}

/// Render changed files as a text table.
fn render_changes_table(changes: &[ChangedFile]) -> String {
    let mut out = format!("Changed Files ({} total)\n", changes.len());
    out.push_str("Path                              Op        Size     Checkpoint\n");
    out.push_str("---------------------------------------------------------------\n");
    for c in changes {
        let size_str = match c.size {
            Some(s) => human_size(s),
            None => "-".into(),
        };
        let cp_info = format!(
            "{} ({}, {})",
            c.checkpoint, c.checkpoint_origin, c.checkpoint_age
        );
        out.push_str(&format!(
            "{:<34}{:<10}{:<9}{}\n",
            truncate_path(&c.path, 33),
            c.op,
            size_str,
            cp_info,
        ));
    }
    out
}

/// Truncate a path string to fit a column, adding "..." if too long.
fn truncate_path(path: &str, max: usize) -> String {
    if path.len() <= max {
        path.to_string()
    } else {
        format!("...{}", &path[path.len() - (max - 3)..])
    }
}

/// Extract pagination params (start_index, max_length, format) from arguments.
fn extract_pagination_params(arguments: &Value) -> (usize, usize, &str) {
    let start_index = arguments
        .get("start_index")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let max_length = arguments
        .get("max_length")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_MAX_LENGTH) as usize;
    let format = arguments
        .get("format")
        .and_then(|v| v.as_str())
        .unwrap_or("text");
    (start_index, max_length, format)
}

/// Build paginated MCP response from text content.
fn paginated_response(
    text: &str,
    start_index: usize,
    max_length: usize,
    request_id: Option<Value>,
) -> JsonRpcResponse {
    let (chunk, total, has_more) = paginate(text, start_index, max_length);
    let mut output = String::new();
    if start_index > 0 || has_more {
        output.push_str(&format!(
            "Content length: {total}\nShowing: {start_index}..{}\n",
            start_index + chunk.len(),
        ));
        if has_more {
            output.push_str(&format!(
                "Use start_index={} to continue.\n",
                start_index + chunk.len(),
            ));
        }
        output.push('\n');
    }
    output.push_str(&chunk);
    tool_ok(request_id, &output)
}

fn tool_ok(id: Option<Value>, text: &str) -> JsonRpcResponse {
    JsonRpcResponse::ok(
        id,
        serde_json::json!({
            "content": [{"type": "text", "text": text}]
        }),
    )
}

/// Handle `snapshots_changes` tool call.
pub fn handle_list_changed_files(
    arguments: &Value,
    scheduler: &AutoSnapshotScheduler,
    workspace_root: &Path,
    request_id: Option<Value>,
) -> JsonRpcResponse {
    let snapshots = scheduler.list_snapshots();
    if snapshots.is_empty() {
        return tool_ok(request_id, "No checkpoints available yet.");
    }

    let changes = collect_changes(scheduler, workspace_root);
    let (start_index, max_length, format) = extract_pagination_params(arguments);

    if format == "json" {
        // JSON output is machine-readable -- return full array without pagination
        // headers that would break JSON parsing.
        let json = serde_json::to_string(&changes).unwrap_or_else(|_| "[]".into());
        return tool_ok(request_id, &json);
    }

    let text = render_changes_table(&changes);
    paginated_response(&text, start_index, max_length, request_id)
}

/// Handle `snapshots_revert` tool call.
pub fn handle_revert_file(
    arguments: &Value,
    scheduler: &AutoSnapshotScheduler,
    workspace_root: &Path,
    request_id: Option<Value>,
    db: Option<&Arc<capsem_logger::DbWriter>>,
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
        // Use std::fs::copy (not clone_file) because the destination is in the
        // VirtioFS shared workspace. APFS clonefile is metadata-only and may not
        // propagate through VirtioFS immediately, causing stale reads in the guest.
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

    // Log the revert as a file event in the session DB.
    if let Some(db) = db {
        let file_action = if action == "restored" {
            capsem_logger::FileAction::Restored
        } else {
            capsem_logger::FileAction::Deleted
        };
        let size = if action == "restored" {
            std::fs::symlink_metadata(&current_file).ok().map(|m| m.len())
        } else {
            None
        };
        db.try_write(capsem_logger::WriteOp::FileEvent(capsem_logger::FileEvent {
            timestamp: SystemTime::now(),
            action: file_action,
            path: format!("{} (from {})", path_str, cp_str_owned),
            size,
        }));
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

/// Summarize changes as compact "+N, ~N, -N" string.
fn format_change_summary(changes: &[Value]) -> String {
    let mut created = 0u32;
    let mut modified = 0u32;
    let mut deleted = 0u32;
    for c in changes {
        match c["op"].as_str().unwrap_or("") {
            "new" => created += 1,
            "modified" => modified += 1,
            "deleted" => deleted += 1,
            _ => {}
        }
    }
    let mut parts = Vec::new();
    if created > 0 {
        parts.push(format!("+{created}"));
    }
    if modified > 0 {
        parts.push(format!("~{modified}"));
    }
    if deleted > 0 {
        parts.push(format!("-{deleted}"));
    }
    if parts.is_empty() {
        "(none)".into()
    } else {
        parts.join(", ")
    }
}

/// Render snapshot list as a text table.
fn render_snapshots_table(
    entries: &[serde_json::Value],
    manual_available: usize,
) -> String {
    let mut out = format!(
        "Snapshots ({} total, {} manual slots available)\n",
        entries.len(),
        manual_available,
    );
    out.push_str("Checkpoint  Origin  Name            Age          Hash          Files  Changes\n");
    out.push_str("----------------------------------------------------------------------------\n");
    for e in entries {
        let cp = e["checkpoint"].as_str().unwrap_or("-");
        let origin = e["origin"].as_str().unwrap_or("-");
        let name = e["name"].as_str().unwrap_or("-");
        let age = e["age"].as_str().unwrap_or("-");
        let hash = e["hash"].as_str().map(|h| &h[..h.len().min(12)]).unwrap_or("-");
        let files = e["files_count"].as_u64().unwrap_or(0);
        let changes = e["changes"].as_array().map(|a| a.as_slice()).unwrap_or(&[]);
        let summary = format_change_summary(changes);
        out.push_str(&format!(
            "{:<12}{:<8}{:<16}{:<13}{:<14}{:<7}{}\n",
            cp,
            origin,
            truncate_path(name, 15),
            age,
            hash,
            files,
            summary,
        ));
    }
    out
}

/// Collect snapshot entries as JSON values (for both text and json rendering).
fn collect_snapshot_entries(
    scheduler: &AutoSnapshotScheduler,
) -> Vec<serde_json::Value> {
    let mut snapshots = scheduler.list_snapshots();
    // list_snapshots returns newest-first; reverse to walk oldest-first.
    snapshots.reverse();

    let mut prev_files: HashMap<String, FileEntry> = HashMap::new();
    let mut entries: Vec<serde_json::Value> = Vec::new();

    for s in &snapshots {
        let snap_files = collect_files(&s.workspace_path);
        let origin = match s.origin {
            SnapshotOrigin::Auto => "auto",
            SnapshotOrigin::Manual => "manual",
        };

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

    // Return newest-first.
    entries.reverse();
    entries
}

/// Handle `snapshots_list` tool call -- return all snapshot metadata with per-snapshot diffs.
///
/// Changes are computed vs the PREVIOUS snapshot (oldest-first), not vs current workspace.
/// This shows what changed AT the time of each snapshot.
pub fn handle_list_snapshots(
    arguments: &Value,
    scheduler: &AutoSnapshotScheduler,
    _workspace_root: &Path,
    request_id: Option<Value>,
) -> JsonRpcResponse {
    let entries = collect_snapshot_entries(scheduler);
    let (start_index, max_length, format) = extract_pagination_params(arguments);

    let text = if format == "json" {
        let summary = serde_json::json!({
            "snapshots": entries,
            "auto_max": scheduler.max_auto(),
            "manual_max": scheduler.max_manual(),
            "manual_available": scheduler.available_manual_slots(),
        });
        summary.to_string()
    } else {
        render_snapshots_table(&entries, scheduler.available_manual_slots())
    };

    paginated_response(&text, start_index, max_length, request_id)
}

/// Compute changes between two snapshots: what's new/modified/deleted in `current` vs `prev`.
fn compute_changes_vs_previous(
    current: &HashMap<String, FileEntry>,
    prev: &HashMap<String, FileEntry>,
) -> Vec<Value> {
    let mut changes = Vec::new();

    // New: in current but not in prev.
    for (path, entry) in current {
        if !prev.contains_key(path) {
            changes.push(serde_json::json!({"path": path, "op": "new", "size": entry.size, "is_symlink": entry.is_symlink}));
        }
    }

    // Deleted: in prev but not in current.
    for (path, entry) in prev {
        if !current.contains_key(path) {
            changes.push(serde_json::json!({"path": path, "op": "deleted", "is_symlink": entry.is_symlink}));
        }
    }

    // Modified: in both but different size.
    for (path, cur_entry) in current {
        if let Some(prev_entry) = prev.get(path) {
            if cur_entry.size != prev_entry.size {
                changes.push(serde_json::json!({"path": path, "op": "modified", "size": cur_entry.size, "is_symlink": cur_entry.is_symlink}));
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

/// Handle `snapshots_compact` tool call -- merge multiple snapshots into one.
pub fn handle_snapshots_compact(
    arguments: &Value,
    scheduler: &mut AutoSnapshotScheduler,
    request_id: Option<Value>,
) -> JsonRpcResponse {
    let checkpoints = match arguments.get("checkpoints").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => return JsonRpcResponse::err(request_id, -32602, "missing 'checkpoints' array"),
    };

    let name = arguments.get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let name = if name.is_empty() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        format!("compacted_{now}")
    } else {
        if let Err(e) = validate_snapshot_name(&name) {
            return JsonRpcResponse::err(request_id, -32602, format!("invalid name: {e}"));
        }
        name
    };

    // Parse checkpoint IDs.
    let mut slots = Vec::new();
    for cp in checkpoints {
        let cp_str = match cp.as_str() {
            Some(s) => s,
            None => return JsonRpcResponse::err(request_id, -32602, "checkpoint must be a string"),
        };
        match parse_checkpoint(cp_str) {
            Ok(slot) => slots.push(slot),
            Err(e) => return JsonRpcResponse::err(request_id, -32602, e),
        }
    }

    let deleted_cps: Vec<String> = slots.iter().map(|s| format!("cp-{s}")).collect();

    match scheduler.compact_snapshots(&slots, &name) {
        Ok(result) => {
            let files_count = collect_files(&result.workspace_path).len();
            JsonRpcResponse::ok(
                request_id,
                serde_json::json!({
                    "content": [{"type": "text", "text": serde_json::json!({
                        "compacted": true,
                        "checkpoint": format!("cp-{}", result.slot),
                        "name": name,
                        "hash": result.hash,
                        "merged_count": deleted_cps.len(),
                        "deleted_checkpoints": deleted_cps,
                        "files_count": files_count,
                    }).to_string()}]
                }),
            )
        }
        Err(e) => JsonRpcResponse::err(request_id, -32603, format!("{e}")),
    }
}

#[cfg(test)]
mod tests {
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
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "version TWO -- longer content here");

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
        std::fs::write(ws.join("modify_me.txt"), "after -- different length").unwrap();
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

        let args = serde_json::json!({"format": "json"});
        let resp = handle_list_snapshots(&args, &sched, &ws, Some(serde_json::json!(1)));
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
        let resp = handle_revert_file(&args, &sched, &ws, Some(serde_json::json!(1)), None);
        assert!(resp.error.is_some());
        let err_msg = &resp.error.unwrap().message;
        assert!(err_msg.contains("no snapshot contains this file"), "unexpected error: {err_msg}");
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
        assert!(text.contains("start_index="), "missing pagination hint: {text}");
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
        assert!(!page2.starts_with("Changed Files"), "page 2 should not repeat the header");
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
        assert!(text.contains("start_index="), "should paginate at max_length=200");
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

        assert!(!text.contains("start_index="), "should not paginate small results: {text}");
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
        let changes: Vec<Value> = serde_json::from_str(&text)
            .expect("format=json should return valid JSON array");
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
        assert!(text.contains("Checkpoint"), "missing Checkpoint column: {text}");
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
        let summary: Value = serde_json::from_str(&text)
            .expect("format=json should return valid JSON");
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
        let content = result["content"].as_array().expect("result must have content array");
        assert!(!content.is_empty(), "content must not be empty");
        let text = content[0]["text"].as_str().expect("content[0] must have text string");

        // text must be valid JSON with the expected shape.
        let data: Value = serde_json::from_str(text)
            .expect("content text must be valid JSON when format=json");

        // Top-level fields the frontend depends on.
        assert!(data["snapshots"].is_array(), "must have snapshots array");
        assert!(data["auto_max"].is_number(), "must have auto_max number");
        assert!(data["manual_max"].is_number(), "must have manual_max number");
        assert!(data["manual_available"].is_number(), "must have manual_available number");

        // Each snapshot must have the fields SnapshotsTab.svelte reads.
        let snaps = data["snapshots"].as_array().unwrap();
        assert!(snaps.len() >= 2, "should have at least 2 snapshots");
        for snap in snaps {
            assert!(snap["checkpoint"].is_string(), "snapshot must have checkpoint: {snap}");
            assert!(snap["slot"].is_number(), "snapshot must have slot: {snap}");
            assert!(snap["origin"].is_string(), "snapshot must have origin: {snap}");
            // name and hash can be null.
            assert!(snap["age"].is_string(), "snapshot must have age: {snap}");
            assert!(snap["files_count"].is_number(), "snapshot must have files_count: {snap}");
            assert!(snap["changes"].is_array(), "snapshot must have changes array: {snap}");
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
        assert!(link_size < 100, "symlink size should be small (target path), not {link_size}");
    }
}
