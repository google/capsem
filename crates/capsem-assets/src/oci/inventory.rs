//! Physical cache usage is independent of receipt readiness and ownership.

use std::{collections::HashSet, os::unix::fs::MetadataExt};

use super::{CacheReceipt, CacheSnapshot};
use anyhow::{Context, Result};
use capsem_foundation::unix::contained::{ContainedDir, EntryKind};

/// Metadata observation only. Rows and reclaim estimates grant neither
/// readiness nor deletion authority; an apply operation revalidates its plan.
#[derive(Clone, Debug)]
pub struct CacheInventory {
    pub usage: CacheUsage,
    pub logical_bytes: u64,
    pub associated_allocated_bytes: u64,
    pub control_allocated_bytes: u64,
    pub unassociated_allocated_bytes: u64,
    pub unmanaged_allocated_bytes: u64,
    /// Every receipt-shaped entry decoded; this is not full image verification.
    pub reference_graph_complete: bool,
    pub invalid_receipts: u64,
    pub images: Vec<ImageInventory>,
}

#[derive(Clone, Debug)]
pub struct ImageInventory {
    pub receipt: CacheReceipt,
    pub snapshot: CacheSnapshot,
    pub logical_bytes: u64,
    pub allocated_bytes: u64,
    pub shared_allocated_bytes: u64,
    pub reclaimable_allocated_bytes: u64,
    pub missing_blobs: u64,
    /// Lock state at collection time. Kernel lease changes need not emit a
    /// filesystem event; removal always rechecks live ownership and locks.
    pub busy: bool,
}

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
    measure_bounded(root, usize::MAX)
}

pub(super) fn measure_bounded(root: &ContainedDir, maximum_entries: usize) -> Result<CacheUsage> {
    let mut remaining = maximum_entries
        .checked_sub(1)
        .context("inventory entry budget exhausted")?;
    let mut usage = CacheUsage::default();
    let mut seen = HashSet::new();
    let mut directories = vec![root.try_clone()?];
    while let Some(directory) = directories.pop() {
        let metadata = directory.metadata()?;
        let allocated = metadata.blocks().checked_mul(512).context("OCI allocation overflow")?;
        if !record(&mut usage, &mut seen, (metadata.dev(), metadata.ino()), allocated)? {
            continue;
        }
        directory.visit_entries(|entry| {
            remaining = remaining
                .checked_sub(1)
                .ok_or_else(|| std::io::Error::other("inventory entry budget exhausted"))?;
            if entry.kind == EntryKind::Directory {
                directories.push(directory.descend(&entry.name)?);
            } else {
                record(
                    &mut usage,
                    &mut seen,
                    (entry.identity.dev, entry.identity.ino),
                    entry.allocated,
                )
                .map_err(std::io::Error::other)?;
            }
            Ok(true)
        })?;
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
