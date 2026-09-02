//! Workspace-versus-checkpoint change detection and its text rendering.

use std::collections::HashMap;
use std::path::Path;
use std::time::SystemTime;

use walkdir::WalkDir;

use crate::auto_snapshot::{snapshot_entry_digest, AutoSnapshotScheduler, SnapshotOrigin};

/// Entry from a directory walk: metadata and a streamed content digest.
#[derive(Debug, Clone, Copy)]
pub(super) struct FileEntry {
    pub(super) size: u64,
    pub(super) is_symlink: bool,
    pub(super) digest: Option<blake3::Hash>,
}

impl FileEntry {
    fn differs_from(self, other: Self) -> bool {
        self.size != other.size
            || self.is_symlink != other.is_symlink
            || match (self.digest, other.digest) {
                (Some(current), Some(snapshot)) => current != snapshot,
                _ => true,
            }
    }
}

/// Entry describing a changed file.
#[derive(Debug, serde::Serialize)]
pub(super) struct ChangedFile {
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
pub(super) fn collect_files(root: &Path) -> HashMap<String, FileEntry> {
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
            // Key by the exact UTF-8 path. A lossy conversion collapses distinct
            // non-UTF8 names onto the same replacement string, corrupting change
            // detection; such names are unaddressable through the JSON tool API
            // anyway, so skip them rather than let them collide.
            let Some(rel_str) = rel.to_str().map(str::to_string) else {
                continue;
            };
            // Use symlink_metadata so we don't follow symlinks for size.
            let size = entry.path().symlink_metadata().map(|m| m.len()).unwrap_or(0);
            let is_symlink = ft.is_symlink();
            files.insert(
                rel_str,
                FileEntry {
                    size,
                    is_symlink,
                    digest: snapshot_entry_digest(entry.path(), is_symlink),
                },
            );
        }
    }
    files
}

pub(super) fn age_string(ts: SystemTime) -> String {
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
pub(super) fn collect_changes(scheduler: &AutoSnapshotScheduler, workspace_root: &Path) -> Vec<ChangedFile> {
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

        // Modified: in both but different metadata or content.
        for (path, cur_entry) in &current_files {
            if let Some(snap_entry) = snap_files.get(path) {
                if cur_entry.differs_from(*snap_entry) && seen_paths.insert(path.clone()) {
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
pub(super) fn render_changes_table(changes: &[ChangedFile]) -> String {
    let mut out = format!("Changed Files ({} total)\n", changes.len());
    out.push_str("Path                              Op        Size     Checkpoint\n");
    out.push_str("---------------------------------------------------------------\n");
    for c in changes {
        let size_str = match c.size {
            Some(s) => human_size(s),
            None => "-".into(),
        };
        let cp_info = format!("{} ({}, {})", c.checkpoint, c.checkpoint_origin, c.checkpoint_age);
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
pub(super) fn truncate_path(path: &str, max: usize) -> String {
    let char_count = path.chars().count();
    if char_count <= max {
        return path.to_string();
    }
    // No room for both the ellipsis and content: return the last `max`
    // chars without prefix. Defensive against ill-typed callers; the
    // production call sites pass max = 33 and 15.
    if max <= 3 {
        let skip = char_count - max;
        let byte_offset = path.char_indices().nth(skip).map(|(i, _)| i).unwrap_or(path.len());
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
