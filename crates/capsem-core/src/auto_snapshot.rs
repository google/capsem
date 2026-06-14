//! Snapshot scheduler for VirtioFS sessions.
//!
//! Manages two pools of APFS clonefile snapshots:
//! - **Auto pool** (slots 0..max_auto): rolling ring buffer, taken periodically
//! - **Manual pool** (slots max_auto..max_auto+max_manual): named snapshots
//!   created on-demand via MCP, never auto-culled
//!
//! The AI can diff and revert files against any populated slot via MCP tools.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

/// Whether a snapshot was taken automatically or manually.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SnapshotOrigin {
    Auto,
    Manual,
}

/// Metadata stored per snapshot slot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotMetadata {
    pub slot: usize,
    pub timestamp: String, // ISO 8601
    pub epoch_secs: u64,
    pub epoch_millis: u128,
    pub origin: SnapshotOrigin,
    pub name: Option<String>,
    pub hash: Option<String>, // blake3 of workspace manifest
}

/// Info about a populated snapshot slot.
#[derive(Debug, Clone)]
pub struct SnapshotSlot {
    pub slot: usize,
    pub timestamp: SystemTime,
    pub workspace_path: PathBuf,
    pub origin: SnapshotOrigin,
    pub name: Option<String>,
    pub hash: Option<String>,
    pub files_count: usize,
}

/// Dual-pool snapshot scheduler (auto ring buffer + manual named snapshots).
pub struct AutoSnapshotScheduler {
    session_dir: PathBuf,
    max_auto: usize,
    max_manual: usize,
    interval: Duration,
    next_auto_slot: usize,
    next_manual_slot: usize,
}

impl AutoSnapshotScheduler {
    pub fn new(
        session_dir: PathBuf,
        max_auto: usize,
        max_manual: usize,
        interval: Duration,
    ) -> Self {
        Self {
            session_dir,
            max_auto,
            max_manual,
            interval,
            next_auto_slot: 0,
            next_manual_slot: 0,
        }
    }

    pub fn interval(&self) -> Duration {
        self.interval
    }

    pub fn max_auto(&self) -> usize {
        self.max_auto
    }

    pub fn max_manual(&self) -> usize {
        self.max_manual
    }

    pub fn snapshots_dir(&self) -> PathBuf {
        self.session_dir.join("auto_snapshots")
    }

    fn slot_dir(&self, slot: usize) -> PathBuf {
        self.snapshots_dir().join(slot.to_string())
    }

    fn workspace_dir(&self) -> PathBuf {
        self.session_dir.join("workspace")
    }

    fn system_dir(&self) -> PathBuf {
        self.session_dir.join("system")
    }

    fn ensure_snapshot_storage_outside_workspace(&self) -> anyhow::Result<()> {
        let workspace = self
            .workspace_dir()
            .canonicalize()
            .context("failed to resolve workspace directory for snapshot safety check")?;
        self.ensure_existing_path_outside_workspace(
            &self.snapshots_dir(),
            &workspace,
            "snapshot storage",
        )
    }

    fn ensure_snapshot_path_outside_workspace(
        &self,
        path: &Path,
        label: &str,
    ) -> anyhow::Result<()> {
        let workspace = self
            .workspace_dir()
            .canonicalize()
            .context("failed to resolve workspace directory for snapshot safety check")?;
        self.ensure_existing_path_outside_workspace(path, &workspace, label)
    }

    fn ensure_existing_path_outside_workspace(
        &self,
        path: &Path,
        workspace: &Path,
        label: &str,
    ) -> anyhow::Result<()> {
        if path.exists() {
            let resolved = path
                .canonicalize()
                .with_context(|| format!("failed to resolve {label} path {}", path.display()))?;
            anyhow::ensure!(
                !resolved.starts_with(workspace),
                "{label} resolves inside live workspace: {} -> {}",
                path.display(),
                resolved.display()
            );
        }
        Ok(())
    }

    /// Absolute slot index for auto pool.
    fn auto_slot(&self, idx: usize) -> usize {
        idx
    }

    /// Absolute slot index for manual pool.
    fn manual_slot(&self, idx: usize) -> usize {
        self.max_auto + idx
    }

    /// Total slots (auto + manual).
    fn total_slots(&self) -> usize {
        self.max_auto + self.max_manual
    }

    /// Take an automatic snapshot (auto pool, ring buffer).
    pub fn take_snapshot(&mut self) -> anyhow::Result<SnapshotSlot> {
        let slot = self.auto_slot(self.next_auto_slot);
        let result = self.snapshot_into_slot(slot, SnapshotOrigin::Auto, None)?;
        self.next_auto_slot = (self.next_auto_slot + 1) % self.max_auto;
        info!(slot, origin = "auto", "snapshot taken");
        Ok(result)
    }

    /// Take a named manual snapshot (manual pool).
    pub fn take_named_snapshot(&mut self, name: &str) -> anyhow::Result<SnapshotSlot> {
        anyhow::ensure!(
            self.available_manual_slots() > 0,
            "no manual snapshot slots available (max {})",
            self.max_manual
        );
        let slot = self.manual_slot(self.next_manual_slot);
        // Find next free manual slot (or overwrite oldest if all full).
        // Since we check available_manual_slots > 0, there's always a free one.
        let result =
            self.snapshot_into_slot(slot, SnapshotOrigin::Manual, Some(name.to_string()))?;
        self.next_manual_slot = (self.next_manual_slot + 1) % self.max_manual;
        info!(
            slot,
            origin = "manual",
            name,
            hash = result.hash.as_deref().unwrap_or("-"),
            "snapshot taken"
        );
        Ok(result)
    }

    /// Internal: snapshot workspace + system into a specific slot.
    fn snapshot_into_slot(
        &self,
        slot: usize,
        origin: SnapshotOrigin,
        name: Option<String>,
    ) -> anyhow::Result<SnapshotSlot> {
        let t0 = std::time::Instant::now();
        let slot_dir = self.slot_dir(slot);
        self.ensure_snapshot_storage_outside_workspace()?;
        self.ensure_snapshot_path_outside_workspace(&slot_dir, "snapshot slot")?;

        if slot_dir.exists() {
            std::fs::remove_dir_all(&slot_dir)?;
        }
        std::fs::create_dir_all(&slot_dir)?;

        // Clone workspace.
        let ws_src = self.workspace_dir();
        let ws_dst = slot_dir.join("workspace");
        clone_directory(&ws_src, &ws_dst)?;
        let clone_ws_ms = t0.elapsed().as_millis();

        // Count files + symlinks in workspace (lightweight metadata-only walk).
        let files_count = walkdir::WalkDir::new(&ws_src)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file() || e.file_type().is_symlink())
            .count();

        // Clone system image.
        let sys_src = self.system_dir();
        let sys_dst = slot_dir.join("system");
        if sys_src.exists() {
            clone_directory(&sys_src, &sys_dst)?;
        }
        let clone_sys_ms = t0.elapsed().as_millis() - clone_ws_ms;

        let now = SystemTime::now();
        let since_epoch = now
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default();
        let epoch = since_epoch.as_secs();
        let epoch_millis = since_epoch.as_millis();

        // Compute workspace hash for manual snapshots only.
        // Auto-snapshots skip the hash (rolling ring buffer, never compared by hash).
        let hash = match origin {
            SnapshotOrigin::Manual => {
                if ws_dst.exists() {
                    Some(workspace_hash(&ws_dst))
                } else {
                    None
                }
            }
            SnapshotOrigin::Auto => None,
        };
        let hash_ms = t0.elapsed().as_millis() - clone_ws_ms - clone_sys_ms;

        let meta = SlotMetadata {
            slot,
            timestamp: chrono_like_iso(epoch),
            epoch_secs: epoch,
            epoch_millis,
            origin,
            name: name.clone(),
            hash: hash.clone(),
        };
        let meta_path = slot_dir.join("metadata.json");
        std::fs::write(&meta_path, serde_json::to_string(&meta)?)?;

        let total_ms = t0.elapsed().as_millis();
        debug!(
            slot,
            clone_ws_ms, clone_sys_ms, hash_ms, total_ms, "snapshot_into_slot timing"
        );

        Ok(SnapshotSlot {
            slot,
            timestamp: now,
            workspace_path: ws_dst,
            origin,
            name,
            hash,
            files_count,
        })
    }

    /// List all populated snapshot slots (both pools), newest first.
    pub fn list_snapshots(&self) -> Vec<SnapshotSlot> {
        let mut slots: Vec<(u128, SnapshotSlot)> = Vec::new();
        for i in 0..self.total_slots() {
            let meta_path = self.slot_dir(i).join("metadata.json");
            if let Ok(data) = std::fs::read_to_string(&meta_path) {
                if let Ok(meta) = serde_json::from_str::<SlotMetadata>(&data) {
                    let ts = SystemTime::UNIX_EPOCH + Duration::from_secs(meta.epoch_secs);
                    slots.push((
                        meta.epoch_millis,
                        SnapshotSlot {
                            slot: i,
                            timestamp: ts,
                            workspace_path: self.slot_dir(i).join("workspace"),
                            origin: meta.origin,
                            name: meta.name,
                            hash: meta.hash,
                            files_count: 0,
                        },
                    ));
                }
            }
        }
        slots.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.slot.cmp(&a.1.slot)));
        slots.into_iter().map(|(_, s)| s).collect()
    }

    /// Get a specific snapshot slot by index.
    pub fn get_snapshot(&self, slot: usize) -> Option<SnapshotSlot> {
        if slot >= self.total_slots() {
            return None;
        }
        let meta_path = self.slot_dir(slot).join("metadata.json");
        let data = std::fs::read_to_string(&meta_path).ok()?;
        let meta: SlotMetadata = serde_json::from_str(&data).ok()?;
        let ts = SystemTime::UNIX_EPOCH + Duration::from_secs(meta.epoch_secs);
        Some(SnapshotSlot {
            slot,
            timestamp: ts,
            workspace_path: self.slot_dir(slot).join("workspace"),
            origin: meta.origin,
            name: meta.name,
            hash: meta.hash,
            files_count: 0,
        })
    }

    /// Get metadata for a slot (without constructing full SnapshotSlot).
    pub fn get_metadata(&self, slot: usize) -> Option<SlotMetadata> {
        let meta_path = self.slot_dir(slot).join("metadata.json");
        let data = std::fs::read_to_string(&meta_path).ok()?;
        serde_json::from_str(&data).ok()
    }

    /// Delete a snapshot by slot index.
    pub fn delete_snapshot(&self, slot: usize) -> anyhow::Result<()> {
        anyhow::ensure!(slot < self.total_slots(), "slot {slot} out of range");
        let dir = self.slot_dir(slot);
        anyhow::ensure!(dir.exists(), "slot {slot} is empty");
        self.ensure_snapshot_path_outside_workspace(&dir, "snapshot slot")?;
        std::fs::remove_dir_all(&dir)?;
        debug!(slot, "snapshot deleted");
        Ok(())
    }

    /// How many manual snapshot slots are available.
    pub fn available_manual_slots(&self) -> usize {
        let used = (0..self.max_manual)
            .filter(|i| {
                self.slot_dir(self.manual_slot(*i))
                    .join("metadata.json")
                    .exists()
            })
            .count();
        self.max_manual.saturating_sub(used)
    }

    /// Delete the oldest auto snapshot to free space.
    /// Returns true if a slot was deleted.
    pub fn evict_oldest(&self) -> bool {
        let auto_slots: Vec<_> = self
            .list_snapshots()
            .into_iter()
            .filter(|s| s.origin == SnapshotOrigin::Auto)
            .collect();
        if let Some(oldest) = auto_slots.last() {
            let dir = self.slot_dir(oldest.slot);
            if self
                .ensure_snapshot_path_outside_workspace(&dir, "snapshot slot")
                .is_err()
            {
                return false;
            }
            if std::fs::remove_dir_all(&dir).is_ok() {
                debug!(slot = oldest.slot, "evicted oldest auto-snapshot");
                return true;
            }
        }
        false
    }

    /// Compact multiple snapshots into a single new manual snapshot.
    ///
    /// Merges workspaces oldest-first (newest file version wins).
    /// Deletes all source snapshots after successful compaction.
    pub fn compact_snapshots(
        &mut self,
        slots: &[usize],
        name: &str,
    ) -> anyhow::Result<SnapshotSlot> {
        let t0 = std::time::Instant::now();
        self.ensure_snapshot_storage_outside_workspace()?;
        anyhow::ensure!(!slots.is_empty(), "no snapshots to compact");
        anyhow::ensure!(
            self.available_manual_slots() > 0,
            "no manual snapshot slots available (max {})",
            self.max_manual
        );

        // Validate all slots exist.
        for &slot in slots {
            anyhow::ensure!(
                self.slot_dir(slot).join("metadata.json").exists(),
                "checkpoint cp-{slot} not found"
            );
        }

        // Load metadata for sorting by time.
        let mut metas: Vec<(usize, u128)> = Vec::new();
        for &slot in slots {
            if let Some(meta) = self.get_metadata(slot) {
                metas.push((slot, meta.epoch_millis));
            }
        }
        // Sort oldest-first so newer files overwrite older.
        metas.sort_by_key(|&(_, epoch)| epoch);

        // Build merged workspace in a temp dir within snapshots dir.
        let tmp_dir = self.snapshots_dir().join("_compact_tmp");
        self.ensure_snapshot_path_outside_workspace(&tmp_dir, "snapshot compact temp")?;
        if tmp_dir.exists() {
            std::fs::remove_dir_all(&tmp_dir)?;
        }
        std::fs::create_dir_all(&tmp_dir)?;
        let merged_ws = tmp_dir.join("workspace");
        std::fs::create_dir_all(&merged_ws)?;

        for &(slot, _) in &metas {
            let src_ws = self.slot_dir(slot).join("workspace");
            if !src_ws.exists() {
                continue;
            }
            // Copy all files from this snapshot into merged (overwriting older versions).
            for entry in walkdir::WalkDir::new(&src_ws)
                .follow_links(false)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let rel = match entry.path().strip_prefix(&src_ws) {
                    Ok(r) => r,
                    Err(_) => continue,
                };
                let dst = merged_ws.join(rel);
                if entry.file_type().is_dir() {
                    let _ = std::fs::create_dir_all(&dst);
                } else if entry.file_type().is_symlink() {
                    // Preserve symlinks as symlinks.
                    if let Some(parent) = dst.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let _ = std::fs::remove_file(&dst);
                    if let Ok(link_target) = std::fs::read_link(entry.path()) {
                        let _ = std::os::unix::fs::symlink(&link_target, &dst);
                    }
                } else if entry.file_type().is_file() {
                    if let Some(parent) = dst.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let _ = std::fs::remove_file(&dst);
                    clone_file(entry.path(), &dst)?;
                }
            }
        }

        // Find an available manual slot.
        let target_slot = (0..self.max_manual)
            .map(|i| self.manual_slot(i))
            .find(|&s| !self.slot_dir(s).join("metadata.json").exists())
            .ok_or_else(|| anyhow::anyhow!("no manual snapshot slots available"))?;

        // Create the new snapshot slot.
        let slot_dir = self.slot_dir(target_slot);
        self.ensure_snapshot_path_outside_workspace(&slot_dir, "snapshot slot")?;
        if slot_dir.exists() {
            std::fs::remove_dir_all(&slot_dir)?;
        }
        std::fs::create_dir_all(&slot_dir)?;

        // Move merged workspace into slot.
        std::fs::rename(&merged_ws, slot_dir.join("workspace"))?;
        // Clean up temp dir.
        let _ = std::fs::remove_dir_all(&tmp_dir);

        // Compute hash.
        let hash = workspace_hash(&slot_dir.join("workspace"));

        // Write metadata.
        let now = SystemTime::now();
        let since_epoch = now
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default();
        let epoch = since_epoch.as_secs();
        let epoch_millis = since_epoch.as_millis();
        let meta = SlotMetadata {
            slot: target_slot,
            timestamp: chrono_like_iso(epoch),
            epoch_secs: epoch,
            epoch_millis,
            origin: SnapshotOrigin::Manual,
            name: Some(name.to_string()),
            hash: Some(hash.clone()),
        };
        let meta_path = slot_dir.join("metadata.json");
        std::fs::write(&meta_path, serde_json::to_string_pretty(&meta)?)?;

        // Delete source snapshots.
        for &(slot, _) in &metas {
            let dir = self.slot_dir(slot);
            if dir.exists() {
                self.ensure_snapshot_path_outside_workspace(&dir, "snapshot slot")?;
                let _ = std::fs::remove_dir_all(&dir);
            }
        }

        let total_ms = t0.elapsed().as_millis();
        info!(
            slot = target_slot,
            name,
            merged = metas.len(),
            total_ms,
            "snapshots compacted"
        );

        Ok(SnapshotSlot {
            slot: target_slot,
            origin: SnapshotOrigin::Manual,
            name: Some(name.to_string()),
            hash: Some(hash),
            timestamp: now,
            workspace_path: slot_dir.join("workspace"),
            files_count: 0,
        })
    }
}

/// Compute a blake3 hash of the workspace manifest (sorted file paths + sizes).
/// Includes symlinks: hashes both symlink size and link target path to
/// distinguish symlinks pointing at different targets.
fn workspace_hash(workspace: &Path) -> String {
    let mut entries: BTreeMap<String, u64> = BTreeMap::new();
    let mut symlink_targets: BTreeMap<String, String> = BTreeMap::new();
    for entry in walkdir::WalkDir::new(workspace)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let ft = entry.file_type();
        if !ft.is_file() && !ft.is_symlink() {
            continue;
        }
        if let Ok(rel) = entry.path().strip_prefix(workspace) {
            let rel_str = rel.to_string_lossy().to_string();
            let size = entry
                .path()
                .symlink_metadata()
                .map(|m| m.len())
                .unwrap_or(0);
            entries.insert(rel_str.clone(), size);
            if ft.is_symlink() {
                if let Ok(target) = std::fs::read_link(entry.path()) {
                    symlink_targets.insert(rel_str, target.to_string_lossy().to_string());
                }
            }
        }
    }
    let mut hasher = blake3::Hasher::new();
    for (path, size) in &entries {
        hasher.update(path.as_bytes());
        hasher.update(&size.to_le_bytes());
        // Include symlink target in hash so different targets produce different hashes.
        if let Some(target) = symlink_targets.get(path) {
            hasher.update(target.as_bytes());
        }
    }
    hasher.finalize().to_hex().to_string()
}

/// Trait for directory snapshot backends.
pub trait SnapshotBackend: Send + Sync {
    fn snapshot(&self, source: &Path, dest: &Path) -> anyhow::Result<()>;
}

/// APFS clonefile backend (macOS). Instant copy-on-write via clonefile(2) syscall.
/// Falls back to recursive copy on non-APFS filesystems.
#[cfg(target_os = "macos")]
pub struct ApfsSnapshot;

#[cfg(target_os = "macos")]
impl SnapshotBackend for ApfsSnapshot {
    fn snapshot(&self, source: &Path, dest: &Path) -> anyhow::Result<()> {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let src_c = CString::new(source.as_os_str().as_bytes())
            .map_err(|_| anyhow::anyhow!("source path contains null byte"))?;
        let dst_c = CString::new(dest.as_os_str().as_bytes())
            .map_err(|_| anyhow::anyhow!("dest path contains null byte"))?;

        // Direct clonefile(2) syscall -- no subprocess overhead.
        let ret = unsafe { libc::clonefile(src_c.as_ptr(), dst_c.as_ptr(), 0) };
        if ret == 0 {
            return Ok(());
        }

        let err = std::io::Error::last_os_error();
        match err.raw_os_error() {
            Some(libc::ENOTSUP) | Some(libc::EXDEV) => {
                warn!("clonefile not supported, falling back to recursive copy");
                let status = std::process::Command::new("cp")
                    .args(["-R"])
                    .arg(source)
                    .arg(dest)
                    .status()?;
                anyhow::ensure!(status.success(), "directory copy failed");
                Ok(())
            }
            _ => Err(anyhow::anyhow!("clonefile failed: {err}")),
        }
    }
}

/// Reflink (FICLONE) snapshot backend for Linux.
///
/// Walks the source directory and attempts `ioctl(dst_fd, FICLONE, src_fd)`
/// for each file. On CoW filesystems (Btrfs, XFS) this is instant and
/// zero-copy. On filesystems that don't support reflinks (ext4), falls back
/// to a standard byte copy per file.
#[cfg(target_os = "linux")]
pub struct ReflinkSnapshot;

#[cfg(target_os = "linux")]
impl ReflinkSnapshot {
    /// FICLONE ioctl request number.
    /// Defined in linux/fs.h as _IOW(0x94, 9, int).
    /// On aarch64: direction bits = 0x40000000, size = sizeof(int)=4 << 16,
    /// type = 0x94 << 8, nr = 9  =>  0x40049409.
    const FICLONE: libc::c_ulong = 0x40049409;

    /// Try to reflink a single file. Returns true on success, false if
    /// FICLONE is not supported (caller should fall back to byte copy).
    pub(crate) fn try_reflink(src: &Path, dst: &Path) -> std::io::Result<bool> {
        use std::os::unix::io::AsRawFd;

        let src_file = std::fs::File::open(src)?;
        let dst_file = std::fs::File::create(dst)?;

        // SAFETY: FICLONE takes (dst_fd, FICLONE, src_fd). Both fds are valid
        // open files. The ioctl either clones the data or returns an error.
        let ret = unsafe {
            libc::ioctl(
                dst_file.as_raw_fd(),
                Self::FICLONE as _,
                src_file.as_raw_fd(),
            )
        };

        if ret == 0 {
            // Preserve permissions from source.
            let meta = src_file.metadata()?;
            dst_file.set_permissions(meta.permissions())?;
            Ok(true)
        } else {
            let err = std::io::Error::last_os_error();
            match err.raw_os_error() {
                // EOPNOTSUPP / ENOSYS / EXDEV / EINVAL -- filesystem doesn't support reflinks.
                Some(libc::EOPNOTSUPP | libc::ENOSYS | libc::EXDEV | libc::EINVAL) => {
                    // Remove the empty dst file; caller will do a byte copy.
                    let _ = std::fs::remove_file(dst);
                    Ok(false)
                }
                _ => {
                    let _ = std::fs::remove_file(dst);
                    Err(err)
                }
            }
        }
    }
}

#[cfg(target_os = "linux")]
impl SnapshotBackend for ReflinkSnapshot {
    fn snapshot(&self, source: &Path, dest: &Path) -> anyhow::Result<()> {
        use std::sync::atomic::{AtomicBool, Ordering};

        std::fs::create_dir_all(dest)?;

        // Track whether FICLONE ever succeeded so we can log the strategy once.
        let reflink_supported = AtomicBool::new(false);
        let mut reflink_failed_logged = false;

        for entry in walkdir::WalkDir::new(source)
            .follow_links(false)
            .min_depth(1)
        {
            let entry = entry?;
            let rel = entry.path().strip_prefix(source)?;
            let target = dest.join(rel);

            if entry.file_type().is_dir() {
                std::fs::create_dir_all(&target)?;
            } else if entry.file_type().is_symlink() {
                // Preserve symlinks as symlinks (not their targets).
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let link_target = std::fs::read_link(entry.path())?;
                std::os::unix::fs::symlink(&link_target, &target)?;
            } else if entry.file_type().is_file() {
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                match Self::try_reflink(entry.path(), &target) {
                    Ok(true) => {
                        reflink_supported.store(true, Ordering::Relaxed);
                    }
                    Ok(false) => {
                        if !reflink_failed_logged {
                            info!(
                                path = %entry.path().display(),
                                "FICLONE not supported on this filesystem, falling back to byte copy"
                            );
                            reflink_failed_logged = true;
                        }
                        std::fs::copy(entry.path(), &target)?;
                    }
                    Err(e) => {
                        warn!(
                            path = %entry.path().display(),
                            error = %e,
                            "FICLONE ioctl failed unexpectedly, falling back to byte copy"
                        );
                        std::fs::copy(entry.path(), &target)?;
                    }
                }
            }
        }

        if reflink_supported.load(Ordering::Relaxed) {
            debug!("snapshot completed using reflinks (FICLONE)");
        } else {
            debug!("snapshot completed using byte copy (FICLONE not available)");
        }

        Ok(())
    }
}

/// Return the default snapshot backend for the current platform.
pub fn default_snapshot_backend() -> Box<dyn SnapshotBackend> {
    #[cfg(target_os = "macos")]
    {
        Box::new(ApfsSnapshot)
    }
    #[cfg(target_os = "linux")]
    {
        Box::new(ReflinkSnapshot)
    }
}

/// Clone a directory tree using the platform-appropriate backend.
pub fn clone_directory(src: &Path, dst: &Path) -> anyhow::Result<()> {
    default_snapshot_backend().snapshot(src, dst)
}

/// Clone a single file using platform-appropriate copy-on-write.
///
/// On macOS: uses `cp -c` (APFS clonefile) with fallback to regular copy.
/// On Linux: uses FICLONE ioctl with fallback to `std::fs::copy`.
pub fn clone_file(src: &Path, dst: &Path) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let src_c = CString::new(src.as_os_str().as_bytes())
            .map_err(|_| anyhow::anyhow!("source path contains null byte"))?;
        let dst_c = CString::new(dst.as_os_str().as_bytes())
            .map_err(|_| anyhow::anyhow!("dest path contains null byte"))?;

        let ret = unsafe { libc::clonefile(src_c.as_ptr(), dst_c.as_ptr(), 0) };
        if ret == 0 {
            return Ok(());
        }
        // clonefile not supported (cross-volume, non-APFS) -- fall back to byte copy.
        std::fs::copy(src, dst)?;
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        match ReflinkSnapshot::try_reflink(src, dst) {
            Ok(true) => return Ok(()),
            Ok(false) | Err(_) => {
                std::fs::copy(src, dst)?;
                return Ok(());
            }
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        std::fs::copy(src, dst)?;
        Ok(())
    }
}

/// Calculate the physical disk usage (allocated blocks) of a sandbox session directory.
/// Correctly handles sparse files (like rootfs.img) on Unix platforms.
pub fn sandbox_disk_usage(session_dir: &Path) -> anyhow::Result<u64> {
    let mut total_bytes = 0;
    if !session_dir.exists() {
        return Ok(0);
    }
    for entry in walkdir::WalkDir::new(session_dir) {
        let entry = entry.map_err(|e| anyhow::anyhow!("walkdir failed: {e}"))?;
        let metadata = entry
            .metadata()
            .map_err(|e| anyhow::anyhow!("metadata failed: {e}"))?;
        if metadata.is_file() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                // st_blocks is the number of 512-byte blocks allocated.
                total_bytes += metadata.blocks() * 512;
            }
            #[cfg(not(unix))]
            {
                total_bytes += metadata.len();
            }
        }
    }
    Ok(total_bytes)
}

/// Clone a sandbox's state (system, workspace, session.db) from one session
/// directory to another. The destination gets the `guest/` subdirectory layout
/// with compat symlinks, ready for VirtioFS.
///
/// Performs fsync on rootfs.img before cloning to flush the VirtioFS write-back
/// cache, ensuring the APFS clone captures all guest writes.
///
/// Returns the disk usage of the destination directory in bytes.
pub fn clone_sandbox_state(src_session_dir: &Path, dst_session_dir: &Path) -> anyhow::Result<u64> {
    let sys_src = src_session_dir.join("system");
    let ws_src = src_session_dir.join("workspace");

    // Flush the host page cache for rootfs.img before cloning.
    // Guest writes arrive via VirtioFS and land in the macOS page cache.
    // Without fsync, clonefile() captures stale APFS data, missing
    // recently written overlay changes (e.g. installed packages).
    let rootfs_path = sys_src.join("rootfs.img");
    if rootfs_path.exists() {
        if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&rootfs_path) {
            f.sync_all()
                .context("failed to fsync rootfs.img before clone")?;
        }
    }

    // Clone into guest/ subdirectories matching VirtioFS share layout.
    let guest_dir = dst_session_dir.join("guest");
    std::fs::create_dir_all(&guest_dir)?;

    let sys_dst = guest_dir.join("system");
    let ws_dst = guest_dir.join("workspace");

    if sys_src.exists() {
        clone_directory(&sys_src, &sys_dst).context("failed to clone system dir")?;
    }
    if ws_src.exists() {
        clone_directory(&ws_src, &ws_dst).context("failed to clone workspace dir")?;
    }

    // Compat symlinks so code using session_dir/system still works
    for name in &["system", "workspace"] {
        let link = dst_session_dir.join(name);
        let target = std::path::Path::new("guest").join(name);
        if !link.exists() {
            std::os::unix::fs::symlink(&target, &link)
                .with_context(|| format!("failed to create compat symlink for {name}"))?;
        }
    }

    // Snapshot session.db at session root (host-only, not in guest/).
    //
    // session.db may be in WAL mode while the VM is running. Copying only the
    // main database file can produce a malformed or stale fork because the
    // committed pages may still live in session.db-wal. Ask SQLite to write a
    // coherent standalone image instead.
    let db_src = src_session_dir.join("session.db");
    if db_src.exists() {
        let db_dst = dst_session_dir.join("session.db");
        clone_session_db_snapshot(&db_src, &db_dst).context("failed to snapshot session.db")?;
    }

    Ok(crate::session::disk_usage_bytes(dst_session_dir))
}

fn clone_session_db_snapshot(src: &Path, dst: &Path) -> anyhow::Result<()> {
    if dst.exists() {
        std::fs::remove_file(dst)
            .with_context(|| format!("failed to remove existing {}", dst.display()))?;
    }
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let src_conn = rusqlite::Connection::open_with_flags(
        src,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX
            | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .with_context(|| format!("failed to open source session db {}", src.display()))?;

    let escaped = dst
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('\'', "''");
    src_conn
        .execute_batch(&format!("VACUUM INTO '{}';", escaped))
        .with_context(|| format!("failed to vacuum session db into {}", dst.display()))?;

    let dst_conn = rusqlite::Connection::open_with_flags(
        dst,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("failed to open cloned session db {}", dst.display()))?;
    dst_conn
        .pragma_query_value(None, "quick_check", |row| row.get::<_, String>(0))
        .and_then(|result| {
            if result == "ok" {
                Ok(())
            } else {
                Err(rusqlite::Error::InvalidQuery)
            }
        })
        .context("cloned session db failed quick_check")?;
    Ok(())
}

/// Simple ISO 8601 timestamp from epoch seconds (no chrono dependency).
fn chrono_like_iso(epoch_secs: u64) -> String {
    let ts = time::OffsetDateTime::from_unix_timestamp(epoch_secs as i64)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
    ts.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| format!("{epoch_secs}"))
}

#[cfg(test)]
mod tests;
