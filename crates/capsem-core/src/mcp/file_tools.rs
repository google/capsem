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
use std::io::Read;
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
    "snapshots_changes",
    "snapshots_list",
    "snapshots_revert",
    "snapshots_create",
    "snapshots_delete",
    "snapshots_history",
    "snapshots_compact",
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
            timeout_secs: None,
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
                    },
                    "include_changes": {
                        "type": "boolean",
                        "description": "Include full per-file change arrays. Defaults to false; compact created/edited/deleted counts are always returned."
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
            timeout_secs: None,
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
            timeout_secs: None,
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
            timeout_secs: None,
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
            timeout_secs: None,
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
            timeout_secs: None,
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
            timeout_secs: None,
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

fn checked_child_path(
    root: &Path,
    relative_path: &str,
    label: &str,
) -> Result<std::path::PathBuf, String> {
    let root = root
        .canonicalize()
        .map_err(|e| format!("failed to resolve {label} root: {e}"))?;
    let rel = Path::new(relative_path);
    if let Some(parent) = rel.parent() {
        let mut current = root.clone();
        for component in parent.components() {
            let std::path::Component::Normal(name) = component else {
                return Err(format!("{label} path has invalid component"));
            };
            current.push(name);
            match std::fs::symlink_metadata(&current) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    return Err(format!(
                        "{label} parent contains symlink: {}",
                        current.display()
                    ));
                }
                Ok(meta) if !meta.is_dir() => {
                    return Err(format!(
                        "{label} parent is not a directory: {}",
                        current.display()
                    ));
                }
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
                Err(e) => {
                    return Err(format!(
                        "failed to inspect {label} parent {}: {e}",
                        current.display()
                    ));
                }
            }
        }
    }
    Ok(root.join(rel))
}

fn read_regular_file_no_follow(path: &Path, label: &str) -> Result<Vec<u8>, String> {
    let meta =
        std::fs::symlink_metadata(path).map_err(|e| format!("failed to inspect {label}: {e}"))?;
    if meta.file_type().is_symlink() {
        return Err(format!("{label} is a symlink"));
    }
    if !meta.is_file() {
        return Err(format!("{label} is not a regular file"));
    }

    #[cfg(unix)]
    let mut file = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
            .map_err(|e| format!("failed to open {label} without following symlinks: {e}"))?
    };
    #[cfg(not(unix))]
    let mut file = std::fs::File::open(path).map_err(|e| format!("failed to open {label}: {e}"))?;

    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|e| format!("failed to read {label}: {e}"))?;
    Ok(bytes)
}

/// Validate a snapshot name: alphanumeric + underscore + hyphen, 1-64 chars.
fn validate_snapshot_name(name: &str) -> Result<&str, String> {
    if name.is_empty() || name.len() > 64 {
        return Err("name must be 1-64 characters".into());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
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
            files.insert(
                rel_str,
                FileEntry {
                    size,
                    is_symlink: ft.is_symlink(),
                },
            );
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
fn collect_changes(scheduler: &AutoSnapshotScheduler, workspace_root: &Path) -> Vec<ChangedFile> {
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
///
/// AB-007: counts and slices in characters, not bytes. The previous
/// implementation used `path.len()` and `&path[byte_offset..]`, which panics
/// with "byte index N is not a char boundary" when the suffix offset lands
/// inside a multibyte UTF-8 sequence. Snapshot rendering walks user-supplied
/// paths, so any non-ASCII path could take down the tool.
fn truncate_path(path: &str, max: usize) -> String {
    let char_count = path.chars().count();
    if char_count <= max {
        return path.to_string();
    }
    // No room for both the ellipsis and content: return the last `max`
    // chars without prefix. Defensive against ill-typed callers; the
    // production call sites pass max = 33 and 15.
    if max <= 3 {
        let skip = char_count - max;
        let byte_offset = path
            .char_indices()
            .nth(skip)
            .map(|(i, _)| i)
            .unwrap_or(path.len());
        return path[byte_offset..].to_string();
    }
    let to_take = max - 3;
    let suffix_start_char = char_count - to_take;
    let byte_offset = path
        .char_indices()
        .nth(suffix_start_char)
        .map(|(i, _)| i)
        .unwrap_or(path.len());
    format!("...{}", &path[byte_offset..])
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
    handle_revert_file_with_rules(arguments, scheduler, workspace_root, request_id, db, None)
}

pub fn handle_revert_file_with_rules(
    arguments: &Value,
    scheduler: &AutoSnapshotScheduler,
    workspace_root: &Path,
    request_id: Option<Value>,
    db: Option<&Arc<capsem_logger::DbWriter>>,
    security_rules: Option<&crate::net::policy_config::SecurityRuleSet>,
) -> JsonRpcResponse {
    let (resp, file_event) =
        handle_revert_file_with_security_event(arguments, scheduler, workspace_root, request_id);
    if let (Some(db), Some(file_event)) = (db, file_event) {
        let empty_rules;
        let rules = match security_rules {
            Some(rules) => rules,
            None => {
                empty_rules = crate::net::policy_config::SecurityRuleSet::new(Vec::new());
                &empty_rules
            }
        };
        crate::security_engine::emit_file_security_write_and_rules_blocking(db, rules, file_event);
    }
    resp
}

pub fn handle_revert_file_with_security_event(
    arguments: &Value,
    scheduler: &AutoSnapshotScheduler,
    workspace_root: &Path,
    request_id: Option<Value>,
) -> (JsonRpcResponse, Option<capsem_logger::FileEvent>) {
    let raw_path = match arguments.get("path").and_then(|v| v.as_str()) {
        Some(p) => p,
        None => {
            return (
                JsonRpcResponse::err(request_id, -32602, "missing 'path' argument"),
                None,
            );
        }
    };

    // Normalize and validate path (strips /root/ prefix if present).
    let path_str = match normalize_path(raw_path) {
        Ok(p) => p,
        Err(e) => {
            return (
                JsonRpcResponse::err(request_id, -32602, format!("invalid path: {e}")),
                None,
            );
        }
    };

    // Resolve checkpoint: explicit or auto-select newest containing the file.
    let (slot, cp_str_owned) =
        if let Some(cp_str) = arguments.get("checkpoint").and_then(|v| v.as_str()) {
            let slot = match parse_checkpoint(cp_str) {
                Ok(s) => s,
                Err(e) => return (JsonRpcResponse::err(request_id, -32602, e), None),
            };
            (slot, cp_str.to_string())
        } else {
            // Auto-select: scan snapshots newest-first, find first containing the file.
            let snapshots = scheduler.list_snapshots();
            let found = snapshots.iter().find(|s| {
                checked_child_path(&s.workspace_path, &path_str, "snapshot source")
                    .ok()
                    .and_then(|p| p.symlink_metadata().ok())
                    .is_some()
            });
            match found {
                Some(s) => (s.slot, format!("cp-{}", s.slot)),
                None => {
                    return (
                        JsonRpcResponse::err(request_id, -32602, "no snapshot contains this file"),
                        None,
                    );
                }
            }
        };

    // Get snapshot.
    let snap = match scheduler.get_snapshot(slot) {
        Some(s) => s,
        None => {
            return (
                JsonRpcResponse::err(
                    request_id,
                    -32602,
                    format!("checkpoint {} not found", cp_str_owned),
                ),
                None,
            )
        }
    };

    let snap_file = match checked_child_path(&snap.workspace_path, &path_str, "snapshot source") {
        Ok(path) => path,
        Err(e) => return (JsonRpcResponse::err(request_id, -32602, e), None),
    };
    let current_file = match checked_child_path(workspace_root, &path_str, "workspace target") {
        Ok(path) => path,
        Err(e) => return (JsonRpcResponse::err(request_id, -32602, e), None),
    };

    // Use symlink_metadata to detect presence without following symlinks.
    let snap_exists = snap_file.symlink_metadata().is_ok();
    let current_exists = current_file.symlink_metadata().is_ok();
    let snap_is_symlink = snap_file
        .symlink_metadata()
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false);
    let current_is_symlink = current_file
        .symlink_metadata()
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false);

    let action;
    // Check if file already matches snapshot (no-op): same content AND same permissions.
    // Skip no-op check for symlinks so comparisons never follow a link target.
    if snap_exists && current_exists && !snap_is_symlink && !current_is_symlink {
        if let (Ok(snap_bytes), Ok(cur_bytes)) = (
            read_regular_file_no_follow(&snap_file, "snapshot source"),
            read_regular_file_no_follow(&current_file, "workspace target"),
        ) {
            let same_perms = match (snap_file.metadata(), current_file.metadata()) {
                (Ok(sm), Ok(cm)) => {
                    use std::os::unix::fs::PermissionsExt;
                    sm.permissions().mode() == cm.permissions().mode()
                }
                _ => true, // can't read metadata, assume same
            };
            if snap_bytes == cur_bytes && same_perms {
                return (
                    JsonRpcResponse::err(
                        request_id,
                        -32602,
                        "file already matches snapshot (already current)",
                    ),
                    None,
                );
            }
        }
    } else if !snap_exists && !current_exists {
        return (
            JsonRpcResponse::err(
                request_id,
                -32602,
                "file does not exist in snapshot or workspace",
            ),
            None,
        );
    }

    if snap_exists {
        // File exists in snapshot -- restore it.
        action = "restored";
        if let Some(parent) = current_file.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return (
                    JsonRpcResponse::err(
                        request_id,
                        -32603,
                        format!("failed to create parent directory: {e}"),
                    ),
                    None,
                );
            }
        }
        if snap_is_symlink {
            // Remove existing file/symlink before creating the new symlink.
            if current_exists {
                let _ = std::fs::remove_file(&current_file);
            }
            // Restore symlink: read the link target from the snapshot and recreate it.
            // Security: the symlink target is whatever the guest originally created.
            // This is safe because we only write into the VirtioFS workspace directory;
            // the guest already had the ability to create this exact symlink.
            match std::fs::read_link(&snap_file) {
                Ok(link_target) => {
                    if let Err(e) = std::os::unix::fs::symlink(&link_target, &current_file) {
                        return (
                            JsonRpcResponse::err(
                                request_id,
                                -32603,
                                format!("failed to restore symlink: {e}"),
                            ),
                            None,
                        );
                    }
                }
                Err(e) => {
                    return (
                        JsonRpcResponse::err(
                            request_id,
                            -32603,
                            format!("failed to read symlink from snapshot: {e}"),
                        ),
                        None,
                    );
                }
            }
        } else {
            // Regular file: remove + write + fsync.
            // VirtioFS caches file metadata (size) in the guest kernel.
            // A plain overwrite can leave the guest with a stale cached size,
            // causing truncated reads. Removing first invalidates the dentry;
            // fsync on the new file and its parent dir flushes metadata to
            // the VirtioFS host so the guest sees the correct size.
            let _ = std::fs::remove_file(&current_file);
            let snap_data = match read_regular_file_no_follow(&snap_file, "snapshot source") {
                Ok(d) => d,
                Err(e) => {
                    return (
                        JsonRpcResponse::err(
                            request_id,
                            -32603,
                            format!("failed to read snapshot file safely: {e}"),
                        ),
                        None,
                    );
                }
            };
            {
                use std::io::Write;
                let mut f = match std::fs::File::create(&current_file) {
                    Ok(f) => f,
                    Err(e) => {
                        return (
                            JsonRpcResponse::err(
                                request_id,
                                -32603,
                                format!("failed to create restored file: {e}"),
                            ),
                            None,
                        );
                    }
                };
                if let Err(e) = f.write_all(&snap_data) {
                    return (
                        JsonRpcResponse::err(
                            request_id,
                            -32603,
                            format!("failed to write restored file: {e}"),
                        ),
                        None,
                    );
                }
                let _ = f.sync_all();
            }
            // Fsync parent dir to flush dentry metadata.
            if let Some(parent) = current_file.parent() {
                if let Ok(dir) = std::fs::File::open(parent) {
                    let _ = dir.sync_all();
                }
            }
            // Restore permissions from snapshot.
            if let Ok(snap_meta) = snap_file.metadata() {
                let _ = std::fs::set_permissions(&current_file, snap_meta.permissions());
            }
        }
    } else {
        // File was created after checkpoint -- delete it.
        action = "deleted";
        if current_exists {
            if let Err(e) = std::fs::remove_file(&current_file) {
                return (
                    JsonRpcResponse::err(request_id, -32603, format!("failed to delete file: {e}")),
                    None,
                );
            }
            if let Some(parent) = current_file.parent() {
                if let Ok(dir) = std::fs::File::open(parent) {
                    let _ = dir.sync_all();
                }
            }
        }
    }

    let file_action = if action == "restored" {
        capsem_logger::FileAction::Restored
    } else {
        capsem_logger::FileAction::Deleted
    };
    let size = if action == "restored" {
        std::fs::symlink_metadata(&current_file)
            .ok()
            .map(|m| m.len())
    } else {
        None
    };
    let file_event = capsem_logger::FileEvent {
        event_id: None,
        timestamp: SystemTime::now(),
        action: file_action,
        path: format!("{} (from {})", path_str, cp_str_owned),
        size,
        trace_id: crate::telemetry::ambient_capsem_trace_id(),
        credential_ref: None,
    };

    (
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
        ),
        Some(file_event),
    )
}

fn change_counts(changes: &[Value]) -> (u32, u32, u32) {
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
    (created, modified, deleted)
}

fn change_summary_value(changes: &[Value]) -> Value {
    let (created, edited, deleted) = change_counts(changes);
    serde_json::json!({
        "created": created,
        "edited": edited,
        "deleted": deleted,
        "total": created + edited + deleted,
    })
}

/// Render snapshot list as a text table.
fn render_snapshots_table(entries: &[serde_json::Value], manual_available: usize) -> String {
    let mut out = format!(
        "Snapshots ({} total, {} manual slots available)\n",
        entries.len(),
        manual_available,
    );
    out.push_str("Checkpoint  Origin  Name            Age          Hash          Files  Created  Edited  Deleted\n");
    out.push_str("----------------------------------------------------------------------------------------------\n");
    for e in entries {
        let cp = e["checkpoint"].as_str().unwrap_or("-");
        let origin = e["origin"].as_str().unwrap_or("-");
        let name = e["name"].as_str().unwrap_or("-");
        let age = e["age"].as_str().unwrap_or("-");
        let hash = e["hash"]
            .as_str()
            .map(|h| &h[..h.len().min(12)])
            .unwrap_or("-");
        let files = e["files_count"].as_u64().unwrap_or(0);
        let summary = &e["changes_summary"];
        out.push_str(&format!(
            "{:<12}{:<8}{:<16}{:<13}{:<14}{:<7}{:<9}{:<8}{}\n",
            cp,
            origin,
            truncate_path(name, 15),
            age,
            hash,
            files,
            summary["created"].as_u64().unwrap_or(0),
            summary["edited"].as_u64().unwrap_or(0),
            summary["deleted"].as_u64().unwrap_or(0),
        ));
    }
    out
}

/// Collect snapshot entries as JSON values (for both text and json rendering).
fn collect_snapshot_entries(
    scheduler: &AutoSnapshotScheduler,
    include_changes: bool,
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

        let mut entry = serde_json::json!({
            "checkpoint": format!("cp-{}", s.slot),
            "slot": s.slot,
            "origin": origin,
            "name": s.name,
            "hash": s.hash,
            "age": age_string(s.timestamp),
            "files_count": snap_files.len(),
            "changes_summary": change_summary_value(&changes),
        });
        if include_changes {
            entry["changes"] = Value::Array(changes);
        }
        entries.push(entry);

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
    let include_changes = arguments
        .get("include_changes")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let entries = collect_snapshot_entries(scheduler, include_changes);
    let (start_index, max_length, format) = extract_pagination_params(arguments);

    if format == "json" {
        let summary = serde_json::json!({
            "snapshots": entries,
            "auto_max": scheduler.max_auto(),
            "manual_max": scheduler.max_manual(),
            "manual_available": scheduler.available_manual_slots(),
        });
        return tool_ok(request_id, &summary.to_string());
    }

    let text = render_snapshots_table(&entries, scheduler.available_manual_slots());
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
            changes.push(
                serde_json::json!({"path": path, "op": "deleted", "is_symlink": entry.is_symlink}),
            );
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
                request_id,
                -32602,
                "cannot delete automatic snapshots (managed by scheduler)",
            );
        }
        None => {
            return JsonRpcResponse::err(
                request_id,
                -32602,
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

    let name = arguments
        .get("name")
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
mod tests;
