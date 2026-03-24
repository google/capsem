//! Host-based file monitor for VirtioFS overlay directories.
//!
//! Uses macOS FSEvents (via the `notify` crate) to watch the session's
//! workspace directory on the host filesystem.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tracing::{debug, info};

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

/// Debounce window in milliseconds.
const DEBOUNCE_MS: u64 = 100;

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

/// Debounced entry for a single path.
struct DebouncedEntry {
    action: FileAction,
    deadline: Instant,
}

/// Host-side file system monitor.
///
/// Watches the VirtioFS overlay's `upper/root/` directory using FSEvents
/// and emits FileEvent records to the session database.
pub struct FsMonitor {
    _watcher: RecommendedWatcher,
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

        let mut watcher = notify::recommended_watcher(move |res: Result<Event, _>| {
            if let Ok(event) = res {
                let _ = event_tx.blocking_send(event);
            }
        })?;

        watcher.watch(&watch_dir, RecursiveMode::Recursive)?;
        info!(dir = %watch_dir.display(), "host fs-monitor started");

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

    /// Process notify events with debouncing and write to DB.
    async fn event_loop(
        mut event_rx: mpsc::Receiver<Event>,
        mut shutdown_rx: mpsc::Receiver<()>,
        strip_prefix: PathBuf,
        db: Arc<DbWriter>,
    ) {
        let mut pending: HashMap<String, DebouncedEntry> = HashMap::new();
        let debounce = Duration::from_millis(DEBOUNCE_MS);

        loop {
            let timeout = pending
                .values()
                .map(|e| {
                    e.deadline
                        .saturating_duration_since(Instant::now())
                })
                .min()
                .unwrap_or(Duration::from_millis(100));

            tokio::select! {
                _ = shutdown_rx.recv() => {
                    // Flush remaining
                    for (path, entry) in pending.drain() {
                        Self::emit(&db, &path, entry.action).await;
                    }
                    break;
                }
                event = event_rx.recv() => {
                    let Some(event) = event else { break };
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
                        pending.insert(rel, DebouncedEntry {
                            action,
                            deadline: Instant::now() + debounce,
                        });
                    }
                }
                _ = tokio::time::sleep(timeout) => {
                    // Flush expired entries
                    let now = Instant::now();
                    let expired: Vec<String> = pending
                        .iter()
                        .filter(|(_, e)| e.deadline <= now)
                        .map(|(k, _)| k.clone())
                        .collect();
                    for key in expired {
                        if let Some(entry) = pending.remove(&key) {
                            Self::emit(&db, &key, entry.action).await;
                        }
                    }
                }
            }
        }
        debug!("host fs-monitor stopped");
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
}
