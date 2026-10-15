//! Guest boot/workspace path validation.

use anyhow::{bail, Context, Result};
use std::path::Path;

/// Validate a file path for boot file injection.
///
/// Rejects: empty paths, paths containing NUL bytes, parent-directory components.
pub fn validate_file_path(path: &str) -> Result<()> {
    if path.is_empty() {
        bail!("file path is empty");
    }
    if path.contains('\0') {
        bail!("file path contains NUL byte: {path:?}");
    }
    if Path::new(path)
        .components()
        .any(|component| component == std::path::Component::ParentDir)
    {
        bail!("file path contains a parent-directory component: {path:?}");
    }
    Ok(())
}

/// Validate a file path for runtime file I/O inside the guest workspace.
///
/// Extends [`validate_file_path`] with symlink and containment checks:
/// 1. String-level validation (empty, NUL, parent-directory components)
/// 2. Rejects paths that are themselves symlinks
/// 3. Canonicalizes the resolved path and verifies it stays within `workspace_root`
///
/// For new files (path does not exist yet), the parent directory is checked.
pub fn validate_file_path_safe(path: &str, workspace_root: &Path) -> Result<()> {
    validate_file_path(path)?;

    let full = Path::new(path);

    // Reject if the path itself is a symlink.
    if full
        .symlink_metadata()
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        bail!("path is a symlink: {path:?}");
    }

    let ws_resolved = workspace_root
        .canonicalize()
        .with_context(|| format!("cannot canonicalize workspace root: {}", workspace_root.display()))?;

    if full.exists() {
        // Existing path: canonicalize and check containment.
        let resolved = full
            .canonicalize()
            .with_context(|| format!("cannot canonicalize path: {path:?}"))?;
        if !resolved.starts_with(&ws_resolved) {
            bail!("path resolves outside workspace: {path:?}");
        }
    } else if let Some(parent) = full.parent() {
        // New file: canonicalize parent and check containment.
        if parent.exists() {
            let resolved_parent = parent
                .canonicalize()
                .with_context(|| format!("cannot canonicalize parent: {}", parent.display()))?;
            if !resolved_parent.starts_with(&ws_resolved) {
                bail!("parent resolves outside workspace: {path:?}");
            }
        }
    }

    Ok(())
}
