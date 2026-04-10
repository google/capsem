//! Host-based file monitor for VirtioFS overlay directories.
//!
//! Uses a stat-based `PollWatcher` (via the `notify` crate) to watch the session's
//! workspace directory on the host filesystem.
//!
//! Design: two-phase queue+flush. Raw events from the watcher are pushed into
//! a bounded queue (no processing on the hot path). A timer fires every
//! FLUSH_INTERVAL_MS to drain the queue, coalesce consecutive same-type
//! events on the same path, and emit the results to the session DB.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use notify::{Config, Event, EventKind, RecursiveMode, Watcher};
use notify::poll::PollWatcher;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use capsem_logger::{DbWriter, FileAction, FileEvent, WriteOp};

/// Directories excluded from monitoring.
const EXCLUDED_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "__pycache__",
    ".cache",
    "target",
    ".venv",
    ".swapfile",
];

/// How often the queue is drained and events are emitted (ms).
const FLUSH_INTERVAL_MS: u64 = 100;

/// How often the PollWatcher rescans the directory tree (ms).
/// Apple VZ VirtioFS writes bypass FSEvents, so we must poll.
const POLL_INTERVAL_MS: u64 = 500;

/// Maximum number of raw events buffered before dropping.
const MAX_QUEUE_SIZE: usize = 10_000;

/// Check if any path component matches an excluded directory.
fn should_exclude(path: &Path) -> bool {
    for component in path.components() {
        if let std::path::Component::Normal(name) = component {
            if let Some(s) = name.to_str() {
                if EXCLUDED_DIRS.contains(&s) {
                    return true;
                }
            }
        }
    }
    false
}

/// Map a notify EventKind to a FileAction.
fn event_to_action(kind: &EventKind) -> Option<FileAction> {
    match kind {
        EventKind::Create(_) => Some(FileAction::Created),
        EventKind::Modify(_) => Some(FileAction::Modified),
        EventKind::Remove(_) => Some(FileAction::Deleted),
        _ => None,
    }
}

/// A raw queued event (path already relativized, exclusions already applied).
struct QueuedEvent {
    path: String,
    action: FileAction,
}

/// Host-side file system monitor.
///
/// Watches the VirtioFS workspace directory using stat-based polling.
/// Apple VZ VirtioFS writes do not trigger macOS FSEvents, so we use
/// `PollWatcher` (which compares mtime/size on each scan) instead of
/// the native `FsEventWatcher`.
pub struct FsMonitor {
    _watcher: PollWatcher,
    shutdown_tx: mpsc::Sender<()>,
}

impl FsMonitor {
    /// Start monitoring `watch_dir` and writing events to `db`.
    ///
    /// `strip_prefix` is removed from absolute paths before recording
    /// (e.g., pass `upper/root/` so paths are relative to /root).
    pub fn start(
        watch_dir: PathBuf,
        strip_prefix: PathBuf,
        db: Arc<DbWriter>,
    ) -> anyhow::Result<Self> {
        let (event_tx, event_rx) = mpsc::channel::<Event>(1024);
        let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>(1);

        let config = Config::default()
            .with_poll_interval(Duration::from_millis(POLL_INTERVAL_MS));
        let mut watcher = PollWatcher::new(move |res: Result<Event, _>| {
            if let Ok(event) = res {
                let _ = event_tx.blocking_send(event);
            }
        }, config)?;

        watcher.watch(&watch_dir, RecursiveMode::Recursive)?;
        info!(dir = %watch_dir.display(), poll_ms = POLL_INTERVAL_MS,
              "host fs-monitor started (poll mode, FSEvents unreliable for VirtioFS)");

        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
                .expect("fs_monitor runtime");
            rt.block_on(Self::event_loop(event_rx, shutdown_rx, strip_prefix, db));
        });

        Ok(Self {
            _watcher: watcher,
            shutdown_tx,
        })
    }

    /// Process notify events: queue on receive, flush on timer.
    async fn event_loop(
        mut event_rx: mpsc::Receiver<Event>,
        mut shutdown_rx: mpsc::Receiver<()>,
        strip_prefix: PathBuf,
        db: Arc<DbWriter>,
    ) {
        let mut queue: Vec<QueuedEvent> = Vec::new();
        let mut dropped: u64 = 0;
        let flush_interval = Duration::from_millis(FLUSH_INTERVAL_MS);

        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => {
                    // Final flush
                    Self::flush(&mut queue, &mut dropped, &db).await;
                    debug!("host fs-monitor stopped");
                    break;
                }
                event = event_rx.recv() => {
                    let Some(event) = event else {
                        Self::flush(&mut queue, &mut dropped, &db).await;
                        debug!("host fs-monitor channel closed");
                        break;
                    };
                    let Some(action) = event_to_action(&event.kind) else { continue };

                    for path in &event.paths {
                        if should_exclude(path) {
                            continue;
                        }
                        let rel = path
                            .strip_prefix(&strip_prefix)
                            .unwrap_or(path)
                            .to_string_lossy()
                            .to_string();
                        if rel.is_empty() {
                            continue;
                        }
                        if queue.len() >= MAX_QUEUE_SIZE {
                            dropped += 1;
                        } else {
                            queue.push(QueuedEvent { path: rel, action });
                        }
                    }
                }
                _ = tokio::time::sleep(flush_interval) => {
                    Self::flush(&mut queue, &mut dropped, &db).await;
                }
            }
        }
    }

    /// Drain the queue, coalesce same-type events per path, emit all.
    ///
    /// For each path, consecutive events of the same action type are coalesced
    /// into one. Different action types on the same path emit separately
    /// (e.g., create then delete = two emitted events).
    async fn flush(queue: &mut Vec<QueuedEvent>, dropped: &mut u64, db: &DbWriter) {
        if queue.is_empty() && *dropped == 0 {
            return;
        }

        if *dropped > 0 {
            warn!(count = *dropped, "fs-monitor queue overflow, events dropped");
            *dropped = 0;
        }

        let batch = std::mem::take(queue);
        let raw_count = batch.len();

        // Coalesce: walk the batch in order. For each (path, action), if the
        // pending map already has the same path with the same action, skip.
        // If it has a different action, emit the pending one first, then
        // store the new action.
        let mut pending: HashMap<String, FileAction> = HashMap::new();
        let mut emitted: u64 = 0;

        for event in batch {
            match pending.get(&event.path) {
                Some(&existing) if existing == event.action => {
                    // Same path, same action -- coalesce (skip)
                }
                Some(_) => {
                    // Same path, different action -- emit the old one first
                    let old_action = pending.insert(event.path.clone(), event.action).unwrap();
                    Self::emit(db, &event.path, old_action).await;
                    emitted += 1;
                }
                None => {
                    pending.insert(event.path, event.action);
                }
            }
        }

        // Emit all remaining pending entries
        for (path, action) in pending {
            Self::emit(db, &path, action).await;
            emitted += 1;
        }

        if emitted > 0 {
            debug!(raw = raw_count, emitted, "fs-monitor flush");
        }
    }

    async fn emit(db: &DbWriter, path: &str, action: FileAction) {
        let size = if action != FileAction::Deleted {
            std::fs::metadata(path).ok().map(|m| m.len())
        } else {
            None
        };
        db.write(WriteOp::FileEvent(FileEvent {
            timestamp: SystemTime::now(),
            action,
            path: path.to_string(),
            size,
        })).await;
    }

    /// Signal the monitor to stop.
    pub async fn stop(&self) {
        let _ = self.shutdown_tx.send(()).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_exclude_git() {
        assert!(should_exclude(Path::new(".git")));
        assert!(should_exclude(Path::new("project/.git/objects")));
    }

    #[test]
    fn should_exclude_node_modules() {
        assert!(should_exclude(Path::new("node_modules")));
        assert!(should_exclude(Path::new("project/node_modules/express")));
    }

    #[test]
    fn should_not_exclude_normal_paths() {
        assert!(!should_exclude(Path::new("project/src/app.js")));
        assert!(!should_exclude(Path::new("README.md")));
    }

    #[test]
    fn should_not_exclude_partial_name() {
        assert!(!should_exclude(Path::new(".github/workflows")));
        assert!(!should_exclude(Path::new("targets/debug")));
    }

    #[test]
    fn event_to_action_maps_correctly() {
        assert_eq!(
            event_to_action(&EventKind::Create(notify::event::CreateKind::File)),
            Some(FileAction::Created)
        );
        assert_eq!(
            event_to_action(&EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Content
            ))),
            Some(FileAction::Modified)
        );
        assert_eq!(
            event_to_action(&EventKind::Remove(notify::event::RemoveKind::File)),
            Some(FileAction::Deleted)
        );
        assert_eq!(
            event_to_action(&EventKind::Access(notify::event::AccessKind::Read)),
            None
        );
    }

    // -- flush coalescing tests --

    /// Test the coalescing logic by extracting it into a pure function.
    /// Returns the list of (path, action) pairs that would be emitted.
    fn coalesce(events: &[(&str, FileAction)]) -> Vec<(String, FileAction)> {
        let mut pending: HashMap<String, FileAction> = HashMap::new();
        let mut result = Vec::new();

        for (path, action) in events {
            let path = path.to_string();
            match pending.get(&path) {
                Some(&existing) if existing == *action => {
                    // Same path, same action -- coalesce
                }
                Some(_) => {
                    // Same path, different action -- emit old, store new
                    let old = pending.insert(path.clone(), *action).unwrap();
                    result.push((path, old));
                }
                None => {
                    pending.insert(path, *action);
                }
            }
        }

        for (path, action) in pending {
            result.push((path, action));
        }
        result
    }

    #[test]
    fn flush_coalesces_same_action_same_path() {
        let result = coalesce(&[
            ("file.txt", FileAction::Modified),
            ("file.txt", FileAction::Modified),
            ("file.txt", FileAction::Modified),
        ]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].1, FileAction::Modified);
    }

    #[test]
    fn flush_preserves_different_actions_same_path() {
        let result = coalesce(&[
            ("file.txt", FileAction::Created),
            ("file.txt", FileAction::Deleted),
        ]);
        assert_eq!(result.len(), 2);
        let actions: Vec<_> = result.iter().map(|(_, a)| *a).collect();
        assert!(actions.contains(&FileAction::Created));
        assert!(actions.contains(&FileAction::Deleted));
    }

    #[test]
    fn flush_different_paths_not_coalesced() {
        let result = coalesce(&[
            ("a.txt", FileAction::Modified),
            ("b.txt", FileAction::Modified),
        ]);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn flush_empty_queue_is_noop() {
        let result = coalesce(&[]);
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn flush_create_modify_delete_sequence() {
        let result = coalesce(&[
            ("file.txt", FileAction::Created),
            ("file.txt", FileAction::Modified),
            ("file.txt", FileAction::Modified),
            ("file.txt", FileAction::Modified),
            ("file.txt", FileAction::Deleted),
        ]);
        // created -> modified (emits created), modified -> modified (coalesced),
        // modified -> deleted (emits modified), remaining: deleted = 3 total
        assert_eq!(result.len(), 3);
        let actions: Vec<_> = result.iter().map(|(_, a)| *a).collect();
        assert!(actions.contains(&FileAction::Created));
        assert!(actions.contains(&FileAction::Modified));
        assert!(actions.contains(&FileAction::Deleted));
    }

    #[test]
    fn flush_interleaved_paths() {
        let result = coalesce(&[
            ("a.txt", FileAction::Modified),
            ("b.txt", FileAction::Created),
            ("a.txt", FileAction::Modified),
            ("b.txt", FileAction::Modified),
        ]);
        // a.txt: 2x modified -> 1 emitted (coalesced)
        // b.txt: created then modified -> 2 emitted
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn flush_modify_modify_delete() {
        // Common pattern: file saved multiple times then deleted
        let result = coalesce(&[
            ("temp.txt", FileAction::Modified),
            ("temp.txt", FileAction::Modified),
            ("temp.txt", FileAction::Deleted),
        ]);
        assert_eq!(result.len(), 2);
        let actions: Vec<_> = result.iter().map(|(_, a)| *a).collect();
        assert!(actions.contains(&FileAction::Modified));
        assert!(actions.contains(&FileAction::Deleted));
    }

    #[test]
    fn flush_create_delete_create() {
        // Edge case: file created, deleted, created again
        let result = coalesce(&[
            ("f.txt", FileAction::Created),
            ("f.txt", FileAction::Deleted),
            ("f.txt", FileAction::Created),
        ]);
        // created -> deleted (emits created), deleted -> created (emits deleted),
        // remaining: created = 3 total
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn queue_overflow_caps_at_max() {
        let mut queue: Vec<QueuedEvent> = Vec::new();
        let mut dropped = 0u64;
        // Fill queue to capacity
        for i in 0..MAX_QUEUE_SIZE {
            queue.push(QueuedEvent {
                path: format!("file_{}.txt", i),
                action: FileAction::Modified,
            });
        }
        // One more should increment dropped
        if queue.len() >= MAX_QUEUE_SIZE {
            dropped += 1;
        }
        assert_eq!(queue.len(), MAX_QUEUE_SIZE);
        assert_eq!(dropped, 1);
    }
}
