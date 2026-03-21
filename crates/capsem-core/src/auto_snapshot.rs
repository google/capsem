//! Rolling auto-snapshot scheduler for VirtioFS sessions.
//!
//! Takes periodic APFS clonefile snapshots of the overlay `upper/` directory
//! into a ring buffer of slots. The AI can diff and revert files against any
//! populated slot via MCP tools.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

/// Metadata stored per snapshot slot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotMetadata {
    pub slot: usize,
    pub timestamp: String, // ISO 8601
    pub epoch_secs: u64,
    #[serde(default)]
    pub epoch_millis: u128,
}

/// Info about a populated snapshot slot.
#[derive(Debug, Clone)]
pub struct SnapshotSlot {
    pub slot: usize,
    pub timestamp: SystemTime,
    pub upper_path: PathBuf,
}

/// Rolling ring buffer of APFS clonefile snapshots.
pub struct AutoSnapshotScheduler {
    session_dir: PathBuf,
    max_slots: usize,
    interval: Duration,
    next_slot: usize,
}

impl AutoSnapshotScheduler {
    pub fn new(session_dir: PathBuf, max_slots: usize, interval: Duration) -> Self {
        Self {
            session_dir,
            max_slots,
            interval,
            next_slot: 0,
        }
    }

    pub fn interval(&self) -> Duration {
        self.interval
    }

    pub fn snapshots_dir(&self) -> PathBuf {
        self.session_dir.join("auto_snapshots")
    }

    fn slot_dir(&self, slot: usize) -> PathBuf {
        self.snapshots_dir().join(slot.to_string())
    }

    fn upper_dir(&self) -> PathBuf {
        self.session_dir.join("upper")
    }

    /// Take a snapshot of `upper/` into the next ring buffer slot.
    /// Returns the slot info on success.
    pub fn take_snapshot(&mut self) -> anyhow::Result<SnapshotSlot> {
        let slot = self.next_slot;
        let slot_dir = self.slot_dir(slot);

        // Clear the slot if it already exists (ring buffer overwrite).
        if slot_dir.exists() {
            std::fs::remove_dir_all(&slot_dir)?;
        }
        std::fs::create_dir_all(&slot_dir)?;

        let src = self.upper_dir();
        let dst = slot_dir.join("upper");

        clone_directory(&src, &dst)?;

        let now = SystemTime::now();
        let since_epoch = now
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default();
        let epoch = since_epoch.as_secs();
        let epoch_millis = since_epoch.as_millis();

        // Write metadata.
        let meta = SlotMetadata {
            slot,
            timestamp: chrono_like_iso(epoch),
            epoch_secs: epoch,
            epoch_millis,
        };
        let meta_path = slot_dir.join("metadata.json");
        std::fs::write(&meta_path, serde_json::to_string(&meta)?)?;

        info!(slot, "auto-snapshot taken");

        self.next_slot = (slot + 1) % self.max_slots;

        Ok(SnapshotSlot {
            slot,
            timestamp: now,
            upper_path: dst,
        })
    }

    /// List all populated snapshot slots, newest first.
    pub fn list_snapshots(&self) -> Vec<SnapshotSlot> {
        let mut slots_with_millis: Vec<(u128, SnapshotSlot)> = Vec::new();
        for i in 0..self.max_slots {
            let meta_path = self.slot_dir(i).join("metadata.json");
            if let Ok(data) = std::fs::read_to_string(&meta_path) {
                if let Ok(meta) = serde_json::from_str::<SlotMetadata>(&data) {
                    let ts = SystemTime::UNIX_EPOCH
                        + Duration::from_secs(meta.epoch_secs);
                    slots_with_millis.push((meta.epoch_millis, SnapshotSlot {
                        slot: i,
                        timestamp: ts,
                        upper_path: self.slot_dir(i).join("upper"),
                    }));
                }
            }
        }
        // Newest first. Use epoch_millis for sub-second precision, slot as tiebreaker.
        slots_with_millis.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.slot.cmp(&a.1.slot)));
        slots_with_millis.into_iter().map(|(_, s)| s).collect()
    }

    /// Get a specific snapshot slot by index.
    pub fn get_snapshot(&self, slot: usize) -> Option<SnapshotSlot> {
        if slot >= self.max_slots {
            return None;
        }
        let meta_path = self.slot_dir(slot).join("metadata.json");
        let data = std::fs::read_to_string(&meta_path).ok()?;
        let meta: SlotMetadata = serde_json::from_str(&data).ok()?;
        let ts = SystemTime::UNIX_EPOCH + Duration::from_secs(meta.epoch_secs);
        Some(SnapshotSlot {
            slot,
            timestamp: ts,
            upper_path: self.slot_dir(slot).join("upper"),
        })
    }

    /// Delete the oldest snapshot slot to free space.
    /// Returns true if a slot was deleted.
    pub fn evict_oldest(&self) -> bool {
        let slots = self.list_snapshots();
        if let Some(oldest) = slots.last() {
            let dir = self.slot_dir(oldest.slot);
            if std::fs::remove_dir_all(&dir).is_ok() {
                debug!(slot = oldest.slot, "evicted oldest auto-snapshot");
                return true;
            }
        }
        false
    }
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
        // Fallback: regular recursive copy (non-APFS filesystem).
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
    // Use the time crate which is already a workspace dependency.
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
        std::fs::create_dir_all(session.join("upper/root")).unwrap();
        std::fs::create_dir_all(session.join("auto_snapshots")).unwrap();
        (tmp, session)
    }

    #[test]
    fn take_snapshot_creates_slot() {
        let (_tmp, session) = setup_session_dir();

        // Write a file to upper/root.
        std::fs::write(session.join("upper/root/hello.txt"), "world").unwrap();

        let mut sched = AutoSnapshotScheduler::new(session.clone(), 3, Duration::from_secs(300));
        let slot = sched.take_snapshot().unwrap();

        assert_eq!(slot.slot, 0);
        assert!(slot.upper_path.join("root/hello.txt").exists());
        let content = std::fs::read_to_string(slot.upper_path.join("root/hello.txt")).unwrap();
        assert_eq!(content, "world");

        // Metadata exists.
        let meta_path = session.join("auto_snapshots/0/metadata.json");
        assert!(meta_path.exists());
    }

    #[test]
    fn ring_buffer_wraps_around() {
        let (_tmp, session) = setup_session_dir();
        let mut sched = AutoSnapshotScheduler::new(session.clone(), 2, Duration::from_secs(300));

        std::fs::write(session.join("upper/root/a.txt"), "first").unwrap();
        sched.take_snapshot().unwrap(); // slot 0

        std::fs::write(session.join("upper/root/a.txt"), "second").unwrap();
        sched.take_snapshot().unwrap(); // slot 1

        std::fs::write(session.join("upper/root/a.txt"), "third").unwrap();
        sched.take_snapshot().unwrap(); // slot 0 again (overwrites)

        // Slot 0 should have "third", not "first".
        let content = std::fs::read_to_string(
            session.join("auto_snapshots/0/upper/root/a.txt"),
        )
        .unwrap();
        assert_eq!(content, "third");

        // Slot 1 should still have "second".
        let content = std::fs::read_to_string(
            session.join("auto_snapshots/1/upper/root/a.txt"),
        )
        .unwrap();
        assert_eq!(content, "second");
    }

    #[test]
    fn list_snapshots_newest_first() {
        let (_tmp, session) = setup_session_dir();
        let mut sched = AutoSnapshotScheduler::new(session.clone(), 5, Duration::from_secs(300));

        sched.take_snapshot().unwrap(); // slot 0
        std::thread::sleep(Duration::from_millis(10));
        sched.take_snapshot().unwrap(); // slot 1
        std::thread::sleep(Duration::from_millis(10));
        sched.take_snapshot().unwrap(); // slot 2

        let list = sched.list_snapshots();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].slot, 2); // newest
        assert_eq!(list[2].slot, 0); // oldest
    }

    #[test]
    fn get_snapshot_returns_none_for_empty_slot() {
        let (_tmp, session) = setup_session_dir();
        let sched = AutoSnapshotScheduler::new(session, 3, Duration::from_secs(300));
        assert!(sched.get_snapshot(0).is_none());
        assert!(sched.get_snapshot(99).is_none());
    }

    #[test]
    fn evict_oldest_removes_slot() {
        let (_tmp, session) = setup_session_dir();
        let mut sched = AutoSnapshotScheduler::new(session.clone(), 3, Duration::from_secs(300));

        sched.take_snapshot().unwrap();
        std::thread::sleep(Duration::from_millis(10));
        sched.take_snapshot().unwrap();

        assert_eq!(sched.list_snapshots().len(), 2);
        assert!(sched.evict_oldest());
        assert_eq!(sched.list_snapshots().len(), 1);
        // The remaining one is the newest (slot 1).
        assert_eq!(sched.list_snapshots()[0].slot, 1);
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
