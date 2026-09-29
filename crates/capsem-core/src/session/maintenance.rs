use std::path::Path;

use capsem_foundation::unix::contained::{ContainedDir, EntryKind};

/// Directories deeper than this are not counted. A guest can nest its
/// workspace without limit, and every level of the walk holds a descriptor.
const MAX_DEPTH: usize = 64;

/// Total bytes allocated on disk under a session directory.
///
/// The session directory holds the guest-writable workspace, so the walk goes
/// through descriptors, never paths: it used to stat an entry without following
/// it and then read it by path, and a guest that swapped a directory for a
/// symlink in between made the host walk the link's target on every `/info`.
/// `descend` refuses a symlink atomically, so a swapped entry is skipped. Sparse
/// files count their allocated blocks, not their logical size.
pub fn disk_usage_bytes(sessions_base: &Path) -> u64 {
    ContainedDir::open_root(sessions_base).map_or(0, |root| usage(&root, 0))
}

fn usage(dir: &ContainedDir, depth: usize) -> u64 {
    let mut total = 0u64;
    let mut children = Vec::new();
    // An unreadable directory counts what was read of it; usage is advisory.
    let _ = dir.visit_entries(|entry| {
        // What a session stores is its files; a directory's own blocks are
        // bookkeeping, and an empty session measures zero.
        if entry.kind != EntryKind::Directory {
            total = total.saturating_add(entry.allocated);
        } else if depth < MAX_DEPTH {
            children.push(entry.name);
        }
        Ok(true)
    });
    for name in children {
        if let Ok(child) = dir.descend(&name) {
            total = total.saturating_add(usage(&child, depth + 1));
        }
    }
    total
}

#[cfg(test)]
mod tests;
