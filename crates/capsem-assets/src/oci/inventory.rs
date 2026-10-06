//! Physical cache usage is independent of receipt readiness and ownership.

use std::{collections::HashSet, os::unix::fs::MetadataExt};

use anyhow::{Context, Result};
use capsem_foundation::unix::contained::{ContainedDir, EntryKind};

/// Observed physical usage, including unknown files and cache controls.
/// External writers can change it during traversal; it grants no readiness
/// or deletion authority. Hard links contribute allocation only once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheUsage {
    pub allocated_bytes: u64,
    /// Names observed, including the root and directories.
    pub entries: u64,
    pub unique_inodes: u64,
}

pub(super) fn measure(root: &ContainedDir) -> Result<CacheUsage> {
    let mut usage = CacheUsage::default();
    let mut seen = HashSet::new();
    let mut directories = vec![root.try_clone()?];
    while let Some(directory) = directories.pop() {
        let metadata = directory.metadata()?;
        let allocated = metadata.blocks().checked_mul(512).context("OCI allocation overflow")?;
        if !record(&mut usage, &mut seen, (metadata.dev(), metadata.ino()), allocated)? {
            continue;
        }
        for entry in directory.entries()? {
            if entry.kind == EntryKind::Directory {
                directories.push(directory.descend(&entry.name)?);
            } else {
                record(
                    &mut usage,
                    &mut seen,
                    (entry.identity.dev, entry.identity.ino),
                    entry.allocated,
                )?;
            }
        }
    }
    Ok(usage)
}

fn record(usage: &mut CacheUsage, seen: &mut HashSet<(u64, u64)>, inode: (u64, u64), allocated: u64) -> Result<bool> {
    usage.entries = usage.entries.checked_add(1).context("OCI entry count overflow")?;
    if !seen.insert(inode) {
        return Ok(false);
    }
    usage.unique_inodes = usage.unique_inodes.checked_add(1).context("OCI inode count overflow")?;
    usage.allocated_bytes = usage
        .allocated_bytes
        .checked_add(allocated)
        .context("OCI allocation overflow")?;
    Ok(true)
}

#[cfg(test)]
mod tests;
