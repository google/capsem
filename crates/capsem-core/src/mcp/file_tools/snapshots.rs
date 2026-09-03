//! Snapshot listing, creation, deletion, history, and compaction tools.

use super::*;

/// Render snapshot list as a text table.
pub(super) fn render_snapshots_table(entries: &[serde_json::Value], manual_available: usize) -> String {
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
        let hash = e["hash"].as_str().map(|h| &h[..h.len().min(12)]).unwrap_or("-");
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
pub(super) fn collect_snapshot_entries(
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
pub(super) fn compute_changes_vs_previous(
    current: &HashMap<String, FileEntry>,
    prev: &HashMap<String, FileEntry>,
) -> Vec<Value> {
    let mut changes = Vec::new();

    // New: in current but not in prev.
    for (path, entry) in current {
        if !prev.contains_key(path) {
            changes.push(
                serde_json::json!({"path": path, "op": "new", "size": entry.size, "is_symlink": entry.is_symlink}),
            );
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
                request_id,
                -32602,
                "cannot delete automatic snapshots (managed by scheduler)",
            );
        }
        None => {
            return JsonRpcResponse::err(request_id, -32602, format!("checkpoint {cp_str} not found"));
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
/// Size of `rel` under `root`, or `None` when absent, walking every
/// component without following symlinks. A symlink anywhere on the path, or
/// at the leaf, is refused rather than sized through.
pub(super) fn contained_file_size(root: &Path, rel: &str) -> Result<Option<u64>, String> {
    use crate::contained_fs::{is_symlink_refusal, ContainedDir, EntryKind};
    let rel = Path::new(rel);
    let Some(name) = rel.file_name() else {
        return Err("path names no file".into());
    };
    let parent_rel = rel.parent().unwrap_or_else(|| Path::new(""));
    let root = match ContainedDir::open_root(root) {
        Ok(root) => root,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("open root: {e}")),
    };
    let parent = match root.walk(parent_rel) {
        Ok(parent) => parent,
        Err(e) if is_symlink_refusal(&e) => return Err("path crosses a symlink".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("walk: {e}")),
    };
    match parent.entry_kind(name).map_err(|e| format!("inspect: {e}"))? {
        None => Ok(None),
        Some(EntryKind::File) => parent
            .open_file(name, nix::fcntl::OFlag::O_RDONLY, nix::sys::stat::Mode::empty())
            .and_then(|file| file.metadata())
            .map(|meta| Some(meta.len()))
            .map_err(|e| format!("open: {e}")),
        Some(EntryKind::Directory) => Err("path names a directory".into()),
        Some(EntryKind::Other) => Err("path names a symlink or special file".into()),
    }
}

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

    // Sizes are read through symlink-refusing handles: the workspace is the
    // guest's, and a snapshot may have captured a guest symlink, so a plain
    // `metadata()` would size whatever host file either points at.
    let current_size = match contained_file_size(workspace_root, &path_str) {
        Ok(size) => size,
        Err(e) => return JsonRpcResponse::err(request_id, -32602, format!("invalid path: {e}")),
    };

    let mut versions: Vec<serde_json::Value> = Vec::new();
    let mut prev_size: Option<u64> = None; // None = file didn't exist in previous snap

    for snap in &snapshots {
        let snap_size = match contained_file_size(&snap.workspace_path, &path_str) {
            Ok(size) => size,
            Err(e) => return JsonRpcResponse::err(request_id, -32602, format!("invalid path: {e}")),
        };

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

    let name = arguments.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
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
