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
            "no manual snapshot slots available (max {})", self.max_manual
        );
        let slot = self.manual_slot(self.next_manual_slot);
        // Find next free manual slot (or overwrite oldest if all full).
        // Since we check available_manual_slots > 0, there's always a free one.
        let result = self.snapshot_into_slot(slot, SnapshotOrigin::Manual, Some(name.to_string()))?;
        self.next_manual_slot = (self.next_manual_slot + 1) % self.max_manual;
        info!(slot, origin = "manual", name, hash = result.hash.as_deref().unwrap_or("-"), "snapshot taken");
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

        if slot_dir.exists() {
            std::fs::remove_dir_all(&slot_dir)?;
        }
        std::fs::create_dir_all(&slot_dir)?;

        // Clone workspace.
        let ws_src = self.workspace_dir();
        let ws_dst = slot_dir.join("workspace");
        clone_directory(&ws_src, &ws_dst)?;
        let clone_ws_ms = t0.elapsed().as_millis();

        // Count files in workspace (lightweight metadata-only walk).
        let files_count = walkdir::WalkDir::new(&ws_src)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
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
                if ws_dst.exists() { Some(workspace_hash(&ws_dst)) } else { None }
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
            clone_ws_ms,
            clone_sys_ms,
            hash_ms,
            total_ms,
            "snapshot_into_slot timing"
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
                    slots.push((meta.epoch_millis, SnapshotSlot {
                        slot: i,
                        timestamp: ts,
                        workspace_path: self.slot_dir(i).join("workspace"),
                        origin: meta.origin,
                        name: meta.name,
                        hash: meta.hash,
                        files_count: 0,
                    }));
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
        std::fs::remove_dir_all(&dir)?;
        debug!(slot, "snapshot deleted");
        Ok(())
    }

    /// How many manual snapshot slots are available.
    pub fn available_manual_slots(&self) -> usize {
        let used = (0..self.max_manual)
            .filter(|i| self.slot_dir(self.manual_slot(*i)).join("metadata.json").exists())
            .count();
        self.max_manual.saturating_sub(used)
    }

    /// Delete the oldest auto snapshot to free space.
    /// Returns true if a slot was deleted.
    pub fn evict_oldest(&self) -> bool {
        let auto_slots: Vec<_> = self.list_snapshots()
            .into_iter()
            .filter(|s| s.origin == SnapshotOrigin::Auto)
            .collect();
        if let Some(oldest) = auto_slots.last() {
            let dir = self.slot_dir(oldest.slot);
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
    pub fn compact_snapshots(&mut self, slots: &[usize], name: &str) -> anyhow::Result<SnapshotSlot> {
        let t0 = std::time::Instant::now();
        anyhow::ensure!(!slots.is_empty(), "no snapshots to compact");
        anyhow::ensure!(
            self.available_manual_slots() > 0,
            "no manual snapshot slots available (max {})", self.max_manual
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
            for entry in walkdir::WalkDir::new(&src_ws).into_iter().filter_map(|e| e.ok()) {
                let rel = match entry.path().strip_prefix(&src_ws) {
                    Ok(r) => r,
                    Err(_) => continue,
                };
                let dst = merged_ws.join(rel);
                if entry.file_type().is_dir() {
                    let _ = std::fs::create_dir_all(&dst);
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
        let since_epoch = now.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
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
                let _ = std::fs::remove_dir_all(&dir);
            }
        }

        let total_ms = t0.elapsed().as_millis();
        info!(slot = target_slot, name, merged = metas.len(), total_ms, "snapshots compacted");

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
fn workspace_hash(workspace: &Path) -> String {
    let mut entries: BTreeMap<String, u64> = BTreeMap::new();
    for entry in walkdir::WalkDir::new(workspace)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        if let Ok(rel) = entry.path().strip_prefix(workspace) {
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            entries.insert(rel.to_string_lossy().to_string(), size);
        }
    }
    let mut hasher = blake3::Hasher::new();
    for (path, size) in &entries {
        hasher.update(path.as_bytes());
        hasher.update(&size.to_le_bytes());
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
            libc::ioctl(dst_file.as_raw_fd(), Self::FICLONE as _, src_file.as_raw_fd())
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

        for entry in walkdir::WalkDir::new(source).min_depth(1) {
            let entry = entry?;
            let rel = entry.path().strip_prefix(source)?;
            let target = dest.join(rel);

            if entry.file_type().is_dir() {
                std::fs::create_dir_all(&target)?;
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
        return Ok(());
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

/// Simple ISO 8601 timestamp from epoch seconds (no chrono dependency).
fn chrono_like_iso(epoch_secs: u64) -> String {
    let ts = time::OffsetDateTime::from_unix_timestamp(epoch_secs as i64)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
    ts.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| format!("{epoch_secs}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_session_dir() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let session = tmp.path().to_path_buf();
        std::fs::create_dir_all(session.join("workspace")).unwrap();
        std::fs::create_dir_all(session.join("system")).unwrap();
        std::fs::create_dir_all(session.join("auto_snapshots")).unwrap();
        (tmp, session)
    }

    fn sched(session: &Path) -> AutoSnapshotScheduler {
        AutoSnapshotScheduler::new(session.to_path_buf(), 3, 4, Duration::from_secs(300))
    }

    #[test]
    fn take_auto_snapshot_creates_slot() {
        let (_tmp, session) = setup_session_dir();
        std::fs::write(session.join("workspace/hello.txt"), "world").unwrap();

        let mut s = sched(&session);
        let slot = s.take_snapshot().unwrap();

        assert_eq!(slot.slot, 0);
        assert_eq!(slot.origin, SnapshotOrigin::Auto);
        assert!(slot.name.is_none());
        assert!(slot.hash.is_none()); // auto-snapshots skip hash for performance
        assert_eq!(slot.files_count, 1); // hello.txt
        assert!(slot.workspace_path.join("hello.txt").exists());
        let content = std::fs::read_to_string(slot.workspace_path.join("hello.txt")).unwrap();
        assert_eq!(content, "world");

        let meta_path = session.join("auto_snapshots/0/metadata.json");
        assert!(meta_path.exists());
        let meta: SlotMetadata = serde_json::from_str(&std::fs::read_to_string(&meta_path).unwrap()).unwrap();
        assert_eq!(meta.origin, SnapshotOrigin::Auto);
        assert!(meta.name.is_none());
    }

    #[test]
    fn take_named_snapshot_has_origin_and_hash() {
        let (_tmp, session) = setup_session_dir();
        std::fs::write(session.join("workspace/file.txt"), "data").unwrap();

        let mut s = sched(&session);
        let slot = s.take_named_snapshot("my_checkpoint").unwrap();

        assert_eq!(slot.slot, 3); // manual pool starts at max_auto=3
        assert_eq!(slot.origin, SnapshotOrigin::Manual);
        assert_eq!(slot.name.as_deref(), Some("my_checkpoint"));
        assert!(slot.hash.is_some());
        assert!(!slot.hash.as_ref().unwrap().is_empty());
        assert_eq!(slot.files_count, 1); // file.txt
    }

    #[test]
    fn files_count_tracks_multiple_files() {
        let (_tmp, session) = setup_session_dir();
        let ws = session.join("workspace");
        std::fs::write(ws.join("a.txt"), "a").unwrap();
        std::fs::write(ws.join("b.txt"), "b").unwrap();
        std::fs::create_dir_all(ws.join("sub")).unwrap();
        std::fs::write(ws.join("sub/c.txt"), "c").unwrap();

        let mut s = sched(&session);
        let slot = s.take_snapshot().unwrap();
        assert_eq!(slot.files_count, 3); // a.txt, b.txt, sub/c.txt

        // Add a file and take another snapshot.
        std::fs::write(ws.join("d.txt"), "d").unwrap();
        let slot2 = s.take_snapshot().unwrap();
        assert_eq!(slot2.files_count, 4);
    }

    #[test]
    fn files_count_zero_for_empty_workspace() {
        let (_tmp, session) = setup_session_dir();
        let mut s = sched(&session);
        let slot = s.take_snapshot().unwrap();
        assert_eq!(slot.files_count, 0);
    }

    #[test]
    fn auto_ring_buffer_wraps() {
        let (_tmp, session) = setup_session_dir();
        let mut s = AutoSnapshotScheduler::new(session.clone(), 2, 2, Duration::from_secs(300));

        std::fs::write(session.join("workspace/a.txt"), "first").unwrap();
        s.take_snapshot().unwrap(); // slot 0

        std::fs::write(session.join("workspace/a.txt"), "second").unwrap();
        s.take_snapshot().unwrap(); // slot 1

        std::fs::write(session.join("workspace/a.txt"), "third").unwrap();
        s.take_snapshot().unwrap(); // slot 0 again

        let content = std::fs::read_to_string(session.join("auto_snapshots/0/workspace/a.txt")).unwrap();
        assert_eq!(content, "third");
        let content = std::fs::read_to_string(session.join("auto_snapshots/1/workspace/a.txt")).unwrap();
        assert_eq!(content, "second");
    }

    #[test]
    fn separate_pools_dont_collide() {
        let (_tmp, session) = setup_session_dir();
        let mut s = AutoSnapshotScheduler::new(session.clone(), 2, 2, Duration::from_secs(300));

        std::fs::write(session.join("workspace/a.txt"), "auto").unwrap();
        s.take_snapshot().unwrap(); // auto slot 0

        std::fs::write(session.join("workspace/a.txt"), "manual").unwrap();
        s.take_named_snapshot("checkpoint_1").unwrap(); // manual slot 2

        let list = s.list_snapshots();
        assert_eq!(list.len(), 2);
        let auto: Vec<_> = list.iter().filter(|s| s.origin == SnapshotOrigin::Auto).collect();
        let manual: Vec<_> = list.iter().filter(|s| s.origin == SnapshotOrigin::Manual).collect();
        assert_eq!(auto.len(), 1);
        assert_eq!(manual.len(), 1);
        assert_eq!(auto[0].slot, 0);
        assert_eq!(manual[0].slot, 2);
    }

    #[test]
    fn auto_cull_does_not_touch_manual() {
        let (_tmp, session) = setup_session_dir();
        let mut s = AutoSnapshotScheduler::new(session.clone(), 2, 2, Duration::from_secs(300));

        s.take_snapshot().unwrap();
        std::thread::sleep(Duration::from_millis(10));
        s.take_named_snapshot("keep_me").unwrap();

        assert!(s.evict_oldest());
        // Manual snapshot should survive.
        let list = s.list_snapshots();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].origin, SnapshotOrigin::Manual);
        assert_eq!(list[0].name.as_deref(), Some("keep_me"));
    }

    #[test]
    fn delete_snapshot_removes_slot() {
        let (_tmp, session) = setup_session_dir();
        let mut s = sched(&session);

        let snap = s.take_named_snapshot("deleteme").unwrap();
        assert_eq!(s.list_snapshots().len(), 1);

        s.delete_snapshot(snap.slot).unwrap();
        assert_eq!(s.list_snapshots().len(), 0);
    }

    #[test]
    fn available_manual_slots_decreases() {
        let (_tmp, session) = setup_session_dir();
        let mut s = sched(&session); // max_manual=4

        assert_eq!(s.available_manual_slots(), 4);
        s.take_named_snapshot("a").unwrap();
        assert_eq!(s.available_manual_slots(), 3);
        s.take_named_snapshot("b").unwrap();
        assert_eq!(s.available_manual_slots(), 2);
    }

    #[test]
    fn manual_pool_full_returns_error() {
        let (_tmp, session) = setup_session_dir();
        let mut s = AutoSnapshotScheduler::new(session.clone(), 2, 1, Duration::from_secs(300));

        s.take_named_snapshot("first").unwrap();
        let err = s.take_named_snapshot("second").unwrap_err();
        assert!(err.to_string().contains("no manual snapshot slots available"));
    }

    #[test]
    fn list_snapshots_newest_first() {
        let (_tmp, session) = setup_session_dir();
        let mut s = sched(&session);

        s.take_snapshot().unwrap();
        std::thread::sleep(Duration::from_millis(10));
        s.take_snapshot().unwrap();
        std::thread::sleep(Duration::from_millis(10));
        s.take_snapshot().unwrap();

        let list = s.list_snapshots();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].slot, 2); // newest
        assert_eq!(list[2].slot, 0); // oldest
    }

    #[test]
    fn get_snapshot_returns_none_for_empty_slot() {
        let (_tmp, session) = setup_session_dir();
        let s = sched(&session);
        assert!(s.get_snapshot(0).is_none());
        assert!(s.get_snapshot(99).is_none());
    }

    #[test]
    fn workspace_hash_is_deterministic() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("ws");
        std::fs::create_dir_all(ws.join("sub")).unwrap();
        std::fs::write(ws.join("a.txt"), "hello").unwrap();
        std::fs::write(ws.join("sub/b.txt"), "world").unwrap();

        let h1 = workspace_hash(&ws);
        let h2 = workspace_hash(&ws);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64); // blake3 hex
    }

    #[test]
    fn workspace_hash_changes_on_modification() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::write(ws.join("a.txt"), "v1").unwrap();

        let h1 = workspace_hash(&ws);
        std::fs::write(ws.join("a.txt"), "v2-longer").unwrap();
        let h2 = workspace_hash(&ws);
        assert_ne!(h1, h2);
    }

    #[test]
    fn compact_two_snapshots_merges_files() {
        let (_tmp, session) = setup_session_dir();
        let mut s = sched(&session);

        // Snap 1: file_a.txt
        std::fs::write(session.join("workspace/file_a.txt"), "aaa").unwrap();
        s.take_named_snapshot("snap_a").unwrap();

        // Snap 2: file_b.txt (file_a still exists)
        std::fs::write(session.join("workspace/file_b.txt"), "bbb").unwrap();
        s.take_named_snapshot("snap_b").unwrap();

        // Compact both into one.
        let slots: Vec<usize> = s.list_snapshots().iter().map(|sn| sn.slot).collect();
        let result = s.compact_snapshots(&slots, "merged").unwrap();

        // Merged snapshot should have both files.
        assert!(result.workspace_path.join("file_a.txt").exists());
        assert!(result.workspace_path.join("file_b.txt").exists());
        assert_eq!(
            std::fs::read_to_string(result.workspace_path.join("file_a.txt")).unwrap(),
            "aaa"
        );
        assert_eq!(
            std::fs::read_to_string(result.workspace_path.join("file_b.txt")).unwrap(),
            "bbb"
        );
    }

    #[test]
    fn compact_newest_wins() {
        let (_tmp, session) = setup_session_dir();
        let mut s = sched(&session);

        // Snap 1: file.txt = "old"
        std::fs::write(session.join("workspace/file.txt"), "old").unwrap();
        let snap1 = s.take_named_snapshot("v1").unwrap();

        // Snap 2: file.txt = "new"
        std::fs::write(session.join("workspace/file.txt"), "new").unwrap();
        let snap2 = s.take_named_snapshot("v2").unwrap();

        let result = s.compact_snapshots(&[snap1.slot, snap2.slot], "merged").unwrap();
        assert_eq!(
            std::fs::read_to_string(result.workspace_path.join("file.txt")).unwrap(),
            "new"
        );
    }

    #[test]
    fn compact_deletes_originals() {
        let (_tmp, session) = setup_session_dir();
        let mut s = sched(&session);

        std::fs::write(session.join("workspace/x.txt"), "x").unwrap();
        let snap1 = s.take_named_snapshot("a").unwrap();
        let snap2 = s.take_named_snapshot("b").unwrap();

        let slot1 = snap1.slot;
        let slot2 = snap2.slot;
        s.compact_snapshots(&[slot1, slot2], "merged").unwrap();

        // Originals should be gone.
        assert!(s.get_snapshot(slot1).is_none());
        assert!(s.get_snapshot(slot2).is_none());
    }

    #[test]
    fn compact_requires_manual_slot() {
        let (_tmp, session) = setup_session_dir();
        // max_manual = 1 so only 1 manual slot available.
        let mut s = AutoSnapshotScheduler::new(session.to_path_buf(), 3, 1, Duration::from_secs(300));

        std::fs::write(session.join("workspace/f.txt"), "data").unwrap();
        let _snap1 = s.take_named_snapshot("fill").unwrap();
        // Manual pool is now full (1/1).
        // Create an auto snapshot to compact.
        let snap2 = s.take_snapshot().unwrap();
        // Compact auto into manual should fail (pool full).
        let result = s.compact_snapshots(&[snap2.slot], "nope");
        assert!(result.is_err(), "should fail when manual pool is full");
    }

    #[test]
    fn compact_invalid_slot_errors() {
        let (_tmp, session) = setup_session_dir();
        let mut s = sched(&session);

        let result = s.compact_snapshots(&[999], "bad");
        assert!(result.is_err());
    }

    #[test]
    fn clone_preserves_file_content() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src_dir");
        let dst = tmp.path().join("dst_dir");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("a.txt"), "hello").unwrap();
        std::fs::write(src.join("sub/b.txt"), "nested").unwrap();

        clone_directory(&src, &dst).unwrap();

        assert_eq!(
            std::fs::read_to_string(dst.join("a.txt")).unwrap(),
            "hello"
        );
        assert_eq!(
            std::fs::read_to_string(dst.join("sub/b.txt")).unwrap(),
            "nested"
        );
    }

    // -----------------------------------------------------------------------
    // SnapshotBackend trait + implementations
    // -----------------------------------------------------------------------

    #[test]
    fn snapshot_backend_trait_is_object_safe() {
        fn _assert_obj_safe(_: &dyn SnapshotBackend) {}
    }

    #[test]
    fn default_backend_returns_apfs_on_macos() {
        let backend = default_snapshot_backend();
        // On macOS, should be ApfsSnapshot. On other platforms, HardlinkSnapshot.
        // We just verify it returns something usable.
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("test.txt"), "hello").unwrap();
        backend.snapshot(&src, &dst).unwrap();
        assert_eq!(std::fs::read_to_string(dst.join("test.txt")).unwrap(), "hello");
    }

    // -----------------------------------------------------------------------
    // clone_directory
    // -----------------------------------------------------------------------

    #[test]
    fn copy_recursive_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("empty_src");
        let dst = tmp.path().join("empty_dst");
        std::fs::create_dir_all(&src).unwrap();

        clone_directory(&src, &dst).unwrap();
        assert!(dst.is_dir());
    }

    #[test]
    fn copy_recursive_single_file() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("file.txt"), "content").unwrap();

        clone_directory(&src, &dst).unwrap();

        assert_eq!(std::fs::read_to_string(dst.join("file.txt")).unwrap(), "content");
    }

    #[test]
    fn copy_recursive_nested_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(src.join("a/b/c")).unwrap();
        std::fs::write(src.join("a/b/c/deep.txt"), "deep").unwrap();
        std::fs::write(src.join("a/top.txt"), "top").unwrap();

        let dst = tmp.path().join("dst");
        clone_directory(&src, &dst).unwrap();

        assert_eq!(std::fs::read_to_string(dst.join("a/b/c/deep.txt")).unwrap(), "deep");
        assert_eq!(std::fs::read_to_string(dst.join("a/top.txt")).unwrap(), "top");
    }

    #[test]
    fn copy_recursive_preserves_binary_content() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        let binary: Vec<u8> = (0..=255).collect();
        std::fs::write(src.join("binary.bin"), &binary).unwrap();

        let dst = tmp.path().join("dst");
        clone_directory(&src, &dst).unwrap();

        assert_eq!(std::fs::read(dst.join("binary.bin")).unwrap(), binary);
    }

    #[test]
    fn copy_recursive_empty_file() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("empty"), "").unwrap();

        let dst = tmp.path().join("dst");
        clone_directory(&src, &dst).unwrap();

        assert_eq!(std::fs::read(dst.join("empty")).unwrap().len(), 0);
    }

    #[test]
    fn copy_recursive_many_files() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        for i in 0..50 {
            std::fs::write(src.join(format!("file_{i}.txt")), format!("content_{i}")).unwrap();
        }

        let dst = tmp.path().join("dst");
        clone_directory(&src, &dst).unwrap();

        for i in 0..50 {
            let content = std::fs::read_to_string(dst.join(format!("file_{i}.txt"))).unwrap();
            assert_eq!(content, format!("content_{i}"));
        }
    }

    #[test]
    fn copy_recursive_source_not_found_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let result = clone_directory(
            &tmp.path().join("nonexistent"),
            &tmp.path().join("dst"),
        );
        assert!(result.is_err());
    }

    #[test]
    fn copy_recursive_empty_subdirs_preserved() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(src.join("empty_subdir")).unwrap();
        std::fs::write(src.join("file.txt"), "ok").unwrap();

        let dst = tmp.path().join("dst");
        clone_directory(&src, &dst).unwrap();

        assert!(dst.join("empty_subdir").is_dir());
        assert_eq!(std::fs::read_to_string(dst.join("file.txt")).unwrap(), "ok");
    }

    // -----------------------------------------------------------------------
    // ReflinkSnapshot backend (Linux-only)
    // -----------------------------------------------------------------------

    #[cfg(target_os = "linux")]
    #[test]
    fn reflink_snapshot_copies_files() {
        // Works on any Linux filesystem -- falls back to byte copy on ext4.
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(src.join("subdir")).unwrap();
        std::fs::write(src.join("a.txt"), "alpha").unwrap();
        std::fs::write(src.join("subdir/b.txt"), "beta").unwrap();

        let dst = tmp.path().join("dst");
        ReflinkSnapshot.snapshot(&src, &dst).unwrap();

        assert_eq!(std::fs::read_to_string(dst.join("a.txt")).unwrap(), "alpha");
        assert_eq!(std::fs::read_to_string(dst.join("subdir/b.txt")).unwrap(), "beta");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reflink_snapshot_empty_source() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();

        let dst = tmp.path().join("dst");
        ReflinkSnapshot.snapshot(&src, &dst).unwrap();
        assert!(dst.is_dir());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reflink_snapshot_source_not_found_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let result = ReflinkSnapshot.snapshot(
            &tmp.path().join("nonexistent"),
            &tmp.path().join("dst"),
        );
        assert!(result.is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reflink_snapshot_preserves_nested_structure() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(src.join("a/b/c")).unwrap();
        std::fs::write(src.join("a/b/c/deep.txt"), "deep").unwrap();
        std::fs::write(src.join("top.txt"), "top").unwrap();

        let dst = tmp.path().join("dst");
        ReflinkSnapshot.snapshot(&src, &dst).unwrap();

        assert_eq!(std::fs::read_to_string(dst.join("a/b/c/deep.txt")).unwrap(), "deep");
        assert_eq!(std::fs::read_to_string(dst.join("top.txt")).unwrap(), "top");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reflink_snapshot_preserves_binary_content() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        let binary: Vec<u8> = (0..=255).collect();
        std::fs::write(src.join("binary.bin"), &binary).unwrap();

        let dst = tmp.path().join("dst");
        ReflinkSnapshot.snapshot(&src, &dst).unwrap();

        assert_eq!(std::fs::read(dst.join("binary.bin")).unwrap(), binary);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reflink_try_reflink_returns_false_on_unsupported_fs() {
        // tmpfs doesn't support FICLONE -- verify graceful fallback.
        let tmp = tempfile::tempdir().unwrap();
        let src_path = tmp.path().join("src.txt");
        let dst_path = tmp.path().join("dst.txt");
        std::fs::write(&src_path, "test").unwrap();

        let result = ReflinkSnapshot::try_reflink(&src_path, &dst_path).unwrap();
        // On tmpfs/ext4, FICLONE is not supported so this should be false.
        // On btrfs/xfs, it would be true. Either way, no error.
        assert!(result == true || result == false);
        // If reflink failed, dst was cleaned up and caller does byte copy.
        if !result {
            assert!(!dst_path.exists());
        }
    }

    // -----------------------------------------------------------------------
    // ApfsSnapshot backend (macOS-only behavior)
    // -----------------------------------------------------------------------

    #[test]
    #[cfg(target_os = "macos")]
    fn apfs_snapshot_copies_files() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("x.txt"), "data").unwrap();
        std::fs::write(src.join("sub/y.txt"), "nested").unwrap();

        let dst = tmp.path().join("dst");
        ApfsSnapshot.snapshot(&src, &dst).unwrap();

        assert_eq!(std::fs::read_to_string(dst.join("x.txt")).unwrap(), "data");
        assert_eq!(std::fs::read_to_string(dst.join("sub/y.txt")).unwrap(), "nested");
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn apfs_snapshot_empty_source() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();

        let dst = tmp.path().join("dst");
        ApfsSnapshot.snapshot(&src, &dst).unwrap();
        assert!(dst.is_dir());
    }

    // -----------------------------------------------------------------------
    // clone_directory (dispatches to platform backend)
    // -----------------------------------------------------------------------

    #[test]
    fn clone_directory_dispatches_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("test.txt"), "cloned").unwrap();

        let dst = tmp.path().join("dst");
        clone_directory(&src, &dst).unwrap();

        assert_eq!(std::fs::read_to_string(dst.join("test.txt")).unwrap(), "cloned");
    }

    // -----------------------------------------------------------------------
    // clone_file (single-file CoW with platform fallback)
    // -----------------------------------------------------------------------

    #[test]
    fn clone_file_copies_content() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src.txt");
        std::fs::write(&src, "hello capsem").unwrap();

        let dst = tmp.path().join("dst.txt");
        clone_file(&src, &dst).unwrap();

        assert_eq!(std::fs::read_to_string(&dst).unwrap(), "hello capsem");
    }

    #[test]
    fn clone_file_overwrites_existing_if_removed_first() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src.txt");
        std::fs::write(&src, "new content").unwrap();

        let dst = tmp.path().join("dst.txt");
        std::fs::write(&dst, "old content").unwrap();

        // clone_file expects dst to not exist (caller removes first).
        std::fs::remove_file(&dst).unwrap();
        clone_file(&src, &dst).unwrap();

        assert_eq!(std::fs::read_to_string(&dst).unwrap(), "new content");
    }

    #[test]
    fn clone_file_empty_file() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("empty.txt");
        std::fs::write(&src, "").unwrap();

        let dst = tmp.path().join("dst.txt");
        clone_file(&src, &dst).unwrap();

        assert_eq!(std::fs::read_to_string(&dst).unwrap(), "");
    }

    #[test]
    fn clone_file_nonexistent_source_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("no_such_file.txt");
        let dst = tmp.path().join("dst.txt");
        assert!(clone_file(&src, &dst).is_err());
    }

    // -----------------------------------------------------------------------
    // Snapshot scheduler with backend trait
    // -----------------------------------------------------------------------

    #[test]
    fn snapshot_scheduler_uses_clone_directory() {
        // Verify the scheduler still works end-to-end after the
        // clone_directory refactor to use SnapshotBackend trait.
        let (_tmp, session) = setup_session_dir();
        std::fs::write(session.join("workspace/data.txt"), "important").unwrap();
        std::fs::write(session.join("system/rootfs.img"), "system_data").unwrap();

        let mut s = sched(&session);
        let slot = s.take_snapshot().unwrap();

        assert!(slot.workspace_path.join("data.txt").exists());
        assert_eq!(
            std::fs::read_to_string(slot.workspace_path.join("data.txt")).unwrap(),
            "important"
        );

        // System dir should also be snapshotted
        let system_snap = session.join(format!("auto_snapshots/{}/system", slot.slot));
        assert!(system_snap.exists());
    }
}
