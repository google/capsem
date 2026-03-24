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
        let slot_dir = self.slot_dir(slot);

        if slot_dir.exists() {
            std::fs::remove_dir_all(&slot_dir)?;
        }
        std::fs::create_dir_all(&slot_dir)?;

        // Clone workspace.
        let ws_src = self.workspace_dir();
        let ws_dst = slot_dir.join("workspace");
        clone_directory(&ws_src, &ws_dst)?;

        // Clone system image.
        let sys_src = self.system_dir();
        let sys_dst = slot_dir.join("system");
        if sys_src.exists() {
            clone_directory(&sys_src, &sys_dst)?;
        }

        let now = SystemTime::now();
        let since_epoch = now
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default();
        let epoch = since_epoch.as_secs();
        let epoch_millis = since_epoch.as_millis();

        // Compute workspace hash for all snapshots.
        let hash = if ws_dst.exists() {
            Some(workspace_hash(&ws_dst))
        } else {
            None
        };

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

        Ok(SnapshotSlot {
            slot,
            timestamp: now,
            workspace_path: ws_dst,
            origin,
            name,
            hash,
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

/// Clone a directory tree using macOS APFS clonefile (`cp -c -R`).
/// Falls back to recursive copy on non-APFS.
pub fn clone_directory(src: &Path, dst: &Path) -> anyhow::Result<()> {
    let status = std::process::Command::new("cp")
        .args(["-c", "-R"])
        .arg(src)
        .arg(dst)
        .status()?;

    if !status.success() {
        warn!("APFS clonefile failed, falling back to regular copy");
        let status = std::process::Command::new("cp")
            .args(["-R"])
            .arg(src)
            .arg(dst)
            .status()?;
        anyhow::ensure!(status.success(), "directory copy failed");
    }
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
        assert!(slot.hash.is_some()); // all snapshots compute hash
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
}
