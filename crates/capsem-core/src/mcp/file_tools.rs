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

use capsem_proto::mcp_contracts::{JsonRpcResponse, McpToolDef, ToolAnnotations};
use serde_json::Value;

use crate::auto_snapshot::{AutoSnapshotScheduler, SnapshotOrigin};

use super::builtin_tools::{paginate, pagination_params};

mod changes;
mod snapshots;

use changes::{age_string, collect_changes, collect_files, render_changes_table, truncate_path, FileEntry};
pub use snapshots::{
    handle_delete_snapshot, handle_list_snapshots, handle_snapshot, handle_snapshots_compact, handle_snapshots_history,
};

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

/// Return tool definitions for file tools.
pub fn file_tool_defs() -> Vec<McpToolDef> {
    vec![
        McpToolDef {
            namespaced_name: "snapshots_changes".into(),
            original_name: "snapshots_changes".into(),
            description: Some(
                concat!(
                    "List files that have changed in the workspace compared to automatic checkpoints. ",
                    "Each entry includes the file path, operation (created/modified/deleted), size, ",
                    "and a checkpoint ID that can be passed to snapshots_revert. ",
                    "Shows newest changes first. Output is paginated (default 5000 chars).",
                )
                .into(),
            ),
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
            description: Some(
                concat!(
                    "List all workspace snapshots (automatic and manual). ",
                    "Shows slot index, origin (auto/manual), name, age, blake3 hash, file count, ",
                    "and a compact change summary. Output is paginated (default 5000 chars).",
                )
                .into(),
            ),
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
            description: Some(
                concat!(
                    "Revert a file to its state at a specific checkpoint. ",
                    "Use the checkpoint ID from snapshots_changes output, or omit checkpoint ",
                    "to auto-select the most recent snapshot containing the file. ",
                    "If the file was created after the checkpoint, it is deleted. ",
                    "If the file was modified, it is restored to its checkpoint state. ",
                    "Changes are reflected immediately in the guest via VirtioFS.",
                )
                .into(),
            ),
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
            description: Some(
                concat!(
                    "Create a named workspace snapshot (checkpoint). ",
                    "The snapshot captures the current state of all files and can be used ",
                    "with snapshots_revert to restore files later. Returns the checkpoint ID, ",
                    "a blake3 hash of the workspace, and the number of remaining snapshot slots.",
                )
                .into(),
            ),
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
            description: Some(
                concat!(
                    "Delete a manual snapshot by checkpoint ID. ",
                    "Only manual (named) snapshots can be deleted. ",
                    "Automatic snapshots are managed by the scheduler.",
                )
                .into(),
            ),
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
            description: Some(
                concat!(
                    "Show the history of a specific file across all snapshots. ",
                    "For each snapshot that contains a version of the file, shows the checkpoint, ",
                    "origin, age, size, and whether the file was created, modified, or unchanged. ",
                    "Accepts both relative paths (hello.txt) and absolute guest paths (/root/hello.txt).",
                )
                .into(),
            ),
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
            description: Some(
                concat!(
                    "Compact multiple snapshots into a single new manual snapshot. ",
                    "Merges workspaces with newest-file-wins strategy. ",
                    "Deletes all source snapshots after successful compaction. ",
                    "Frees snapshot slots while preserving file state.",
                )
                .into(),
            ),
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

fn checked_child_path(root: &Path, relative_path: &str, label: &str) -> Result<std::path::PathBuf, String> {
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
                    return Err(format!("{label} parent contains symlink: {}", current.display()));
                }
                Ok(meta) if !meta.is_dir() => {
                    return Err(format!("{label} parent is not a directory: {}", current.display()));
                }
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
                Err(e) => {
                    return Err(format!("failed to inspect {label} parent {}: {e}", current.display()));
                }
            }
        }
    }
    Ok(root.join(rel))
}

/// Mode bits of a trusted snapshot file, defaulting to 0644 if unreadable.
fn snapshot_file_mode(path: &Path) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::symlink_metadata(path)
            .map(|m| m.permissions().mode())
            .unwrap_or(0o644)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        0o644
    }
}

/// Write `data` to a fresh regular file at `path` without following symlinks.
///
/// Uses O_CREAT|O_EXCL (`create_new`) plus O_NOFOLLOW so the write refuses if
/// anything already exists at the path -- including a symlink a guest raced into
/// the VirtioFS-shared workspace between the containment check and here. The
/// mode is set at creation, so there is no follow-prone `set_permissions`
/// afterward. This closes the revert TOCTOU that let a guest redirect a restore
/// to an arbitrary host file.
fn write_regular_file_no_follow(path: &Path, data: &[u8], mode: u32) -> Result<(), String> {
    capsem_foundation::unix::fs::write_new_regular_file_no_follow(path, data, mode)
        .map_err(|error| format!("failed to restore regular file: {error}"))
}

fn read_regular_file_no_follow(path: &Path, label: &str) -> Result<Vec<u8>, String> {
    capsem_foundation::unix::fs::read_regular_file_no_follow(path)
        .map_err(|error| format!("failed to read {label} without following symlinks: {error}"))
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

/// Extract pagination params (start_index, max_length, format) from arguments.
fn extract_pagination_params(arguments: &Value) -> (usize, usize, &str) {
    let (start_index, max_length) = pagination_params(arguments);
    let format = arguments.get("format").and_then(|v| v.as_str()).unwrap_or("text");
    (start_index, max_length, format)
}

/// Build paginated MCP response from text content.
fn paginated_response(text: &str, start_index: usize, max_length: usize, request_id: Option<Value>) -> JsonRpcResponse {
    let (chunk, total, has_more) = paginate(text, start_index, max_length);
    let next_index = start_index.saturating_add(chunk.len());
    let mut output = String::new();
    if start_index > 0 || has_more {
        output.push_str(&format!(
            "Content length: {total}\nShowing: {start_index}..{next_index}\n"
        ));
        if has_more {
            output.push_str(&format!("Use start_index={next_index} to continue.\n"));
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
    let (resp, file_event) = handle_revert_file_with_security_event(arguments, scheduler, workspace_root, request_id);
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
    let (slot, cp_str_owned) = if let Some(cp_str) = arguments.get("checkpoint").and_then(|v| v.as_str()) {
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
                JsonRpcResponse::err(request_id, -32602, format!("checkpoint {} not found", cp_str_owned)),
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
                    JsonRpcResponse::err(request_id, -32602, "file already matches snapshot (already current)"),
                    None,
                );
            }
        }
    } else if !snap_exists && !current_exists {
        return (
            JsonRpcResponse::err(request_id, -32602, "file does not exist in snapshot or workspace"),
            None,
        );
    }

    if snap_exists {
        // File exists in snapshot -- restore it.
        action = "restored";
        if let Some(parent) = current_file.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return (
                    JsonRpcResponse::err(request_id, -32603, format!("failed to create parent directory: {e}")),
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
                            JsonRpcResponse::err(request_id, -32603, format!("failed to restore symlink: {e}")),
                            None,
                        );
                    }
                }
                Err(e) => {
                    return (
                        JsonRpcResponse::err(request_id, -32603, format!("failed to read symlink from snapshot: {e}")),
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
                        JsonRpcResponse::err(request_id, -32603, format!("failed to read snapshot file safely: {e}")),
                        None,
                    );
                }
            };
            // Permissions come from the trusted snapshot side (outside the guest
            // share). Default to 0644 if unreadable.
            let mode = snapshot_file_mode(&snap_file);
            // Write through a no-follow, exclusive create so a guest cannot race
            // a symlink into the just-removed target and redirect the restore.
            if let Err(e) = write_regular_file_no_follow(&current_file, &snap_data, mode) {
                return (
                    JsonRpcResponse::err(request_id, -32603, format!("failed to restore file safely: {e}")),
                    None,
                );
            }
            // Fsync parent dir to flush dentry metadata.
            if let Some(parent) = current_file.parent() {
                if let Ok(dir) = std::fs::File::open(parent) {
                    let _ = dir.sync_all();
                }
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
        std::fs::symlink_metadata(&current_file).ok().map(|m| m.len())
    } else {
        None
    };
    let file_event = capsem_logger::FileEvent {
        event_id: None,
        timestamp: SystemTime::now(),
        action: file_action,
        path: format!("{} (from {})", path_str, cp_str_owned),
        size,
        trace_id: capsem_foundation::telemetry::ambient_capsem_trace_id(),
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

#[cfg(test)]
mod tests;
