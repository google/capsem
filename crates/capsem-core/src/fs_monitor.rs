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

use notify::poll::PollWatcher;
use notify::{Config, Event, EventKind, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use capsem_logger::{DbWriter, FileAction, FileEvent, FileKind};

use crate::credential_broker::{broker_and_log_observations, parse_env_credentials};
use crate::net::ai_traffic::TraceState;
use crate::net::policy_config::SecurityRuleSet;

/// How often the queue is drained and events are emitted (ms).
const FLUSH_INTERVAL_MS: u64 = 100;

/// Floor and ceiling for the PollWatcher rescan cadence (ms).
/// Apple VZ VirtioFS writes bypass FSEvents, so we must poll.
const POLL_INTERVAL_MIN_MS: u64 = 500;
const POLL_INTERVAL_MAX_MS: u64 = 10_000;

/// Scans per interval. One scan stats every entry of the workspace, and the
/// monitor watches every path -- including `node_modules` and `target`, which
/// is the whole point -- so the cost is real and proportional to the tree.
/// Spending at most a tenth of the wall clock on it keeps the monitor from
/// competing with the workload it is watching, whatever the tree's size.
const POLL_SCAN_DUTY_CYCLE: u32 = 10;

/// Poll cadence for a tree whose full stat walk took `scan`.
///
/// Ten scans per interval, floored at 500ms so a small workspace still reacts
/// promptly, capped at 10s so a very large one is still watched. Cost is
/// answered here, never by deciding in advance which paths are not worth
/// recording: `.git/hooks`, `node_modules` and `target` are precisely where a
/// compromise persists.
fn poll_interval_for_scan(scan: Duration) -> Duration {
    (scan * POLL_SCAN_DUTY_CYCLE).clamp(
        Duration::from_millis(POLL_INTERVAL_MIN_MS),
        Duration::from_millis(POLL_INTERVAL_MAX_MS),
    )
}

/// Maximum number of raw events buffered before dropping.
///
/// A 100k-file install between two 100ms flushes must not lose events; a
/// queued row is ~150 bytes, so the bound is ~15MB transient.
const MAX_QUEUE_SIZE: usize = 100_000;

fn path_kind(fs_path: &Path) -> Option<FileKind> {
    let file_type = std::fs::symlink_metadata(fs_path).ok()?.file_type();
    Some(if file_type.is_symlink() {
        FileKind::Symlink
    } else if file_type.is_dir() {
        FileKind::Dir
    } else if file_type.is_file() {
        FileKind::File
    } else {
        FileKind::Other
    })
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

/// A raw queued event (path already relativized; nothing is filtered out).
struct QueuedEvent {
    path: String,
    fs_path: PathBuf,
    action: FileAction,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SnapshotEntry {
    fs_path: PathBuf,
    kind: FileKind,
    len: u64,
    modified: Option<(u64, u32)>,
}

struct WorkspaceState {
    watch_dir: PathBuf,
    strip_prefix: PathBuf,
    snapshot: HashMap<String, SnapshotEntry>,
}

fn snapshot_entry(fs_path: &Path) -> Option<SnapshotEntry> {
    let metadata = std::fs::symlink_metadata(fs_path).ok()?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|value| (value.as_secs(), value.subsec_nanos()));
    Some(SnapshotEntry {
        fs_path: fs_path.to_path_buf(),
        kind: path_kind(fs_path)?,
        len: metadata.len(),
        modified,
    })
}

fn workspace_snapshot(watch_dir: &Path, strip_prefix: &Path) -> HashMap<String, SnapshotEntry> {
    // Every entry, with nothing pruned: a walk that skips a directory is a
    // ledger that lies about it.
    let mut snapshot = HashMap::new();
    for entry in walkdir::WalkDir::new(watch_dir)
        .min_depth(1)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        let fs_path = entry.path();
        let rel = fs_path
            .strip_prefix(strip_prefix)
            .unwrap_or(fs_path)
            .to_string_lossy()
            .to_string();
        if rel.is_empty() {
            continue;
        }
        if let Some(snapshot_entry) = snapshot_entry(fs_path) {
            snapshot.insert(rel, snapshot_entry);
        }
    }
    snapshot
}

/// What the event's path is, as of the moment it is emitted.
///
/// A deletion has nothing left to stat, so the last snapshot entry is the only
/// remaining witness to whether a directory or a file disappeared.
fn resolve_kind(snapshot: &HashMap<String, SnapshotEntry>, path: &str, fs_path: &Path, action: FileAction) -> FileKind {
    let remembered = || snapshot.get(path).map(|entry| entry.kind);
    if action == FileAction::Deleted {
        return remembered().unwrap_or_default();
    }
    path_kind(fs_path).or_else(remembered).unwrap_or_default()
}

fn reconciliation_events(
    previous: &HashMap<String, SnapshotEntry>,
    current: &HashMap<String, SnapshotEntry>,
) -> Vec<QueuedEvent> {
    let mut paths = previous.keys().chain(current.keys()).cloned().collect::<Vec<_>>();
    paths.sort();
    paths.dedup();

    paths
        .into_iter()
        .filter_map(|path| match (previous.get(&path), current.get(&path)) {
            (None, Some(entry)) => Some(QueuedEvent {
                path,
                fs_path: entry.fs_path.clone(),
                action: FileAction::Created,
            }),
            (Some(entry), None) => Some(QueuedEvent {
                path,
                fs_path: entry.fs_path.clone(),
                action: FileAction::Deleted,
            }),
            (Some(before), Some(after)) if before != after => Some(QueuedEvent {
                path,
                fs_path: after.fs_path.clone(),
                action: FileAction::Modified,
            }),
            _ => None,
        })
        .collect()
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
    /// JoinHandle stored so `shutdown_and_join` can sequence "fs_monitor
    /// fully flushed" before the caller tears down the DbWriter. Without
    /// this, the pending-event flush at shutdown raced with the WAL
    /// checkpoint -- the signal-driven explicit-cleanup pattern in
    /// capsem-process relies on fs events landing before the checkpoint.
    join_handle: std::sync::Mutex<Option<std::thread::JoinHandle<()>>>,
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
        security_rules: Arc<std::sync::RwLock<Arc<SecurityRuleSet>>>,
        trace_state: Arc<std::sync::Mutex<TraceState>>,
    ) -> anyhow::Result<Self> {
        // PollWatcher discovers changes asynchronously. Keep an owner-local
        // baseline so shutdown can reconcile changes that exist on disk but
        // have not reached the notify callback yet.
        // The baseline walk stats exactly what each poll scan will stat, so it
        // is also the measurement that sets the cadence.
        let scan_started = std::time::Instant::now();
        let initial_snapshot = workspace_snapshot(&watch_dir, &strip_prefix);
        let scan_duration = scan_started.elapsed();
        let poll_interval = poll_interval_for_scan(scan_duration);
        let workspace_state = WorkspaceState {
            watch_dir: watch_dir.clone(),
            strip_prefix,
            snapshot: initial_snapshot,
        };
        let (event_tx, event_rx) = mpsc::channel::<Event>(1024);
        let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>(1);

        let entries = workspace_state.snapshot.len();
        let config = Config::default().with_poll_interval(poll_interval);
        let mut watcher = PollWatcher::new(
            move |res: Result<Event, _>| {
                if let Ok(event) = res {
                    let _ = event_tx.blocking_send(event);
                }
            },
            config,
        )?;

        watcher.watch(&watch_dir, RecursiveMode::Recursive)?;
        info!(dir = %watch_dir.display(), entries, scan_ms = scan_duration.as_millis(),
              poll_ms = poll_interval.as_millis(),
              "host fs-monitor started (poll mode, FSEvents unreliable for VirtioFS)");

        let join_handle = std::thread::Builder::new()
            .name("capsem-fs-monitor".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_time()
                    .build()
                    .expect("fs_monitor runtime");
                rt.block_on(Self::event_loop(
                    event_rx,
                    shutdown_rx,
                    workspace_state,
                    db,
                    security_rules,
                    trace_state,
                ));
            })
            .expect("failed to spawn fs_monitor thread");

        Ok(Self {
            _watcher: watcher,
            shutdown_tx,
            join_handle: std::sync::Mutex::new(Some(join_handle)),
        })
    }

    /// Signal the event loop to flush and exit, then block until the
    /// worker thread has run its final flush into `DbWriter`. Idempotent.
    /// Call from a blocking context (e.g. `tokio::task::spawn_blocking`).
    pub fn shutdown_and_join(&self) {
        let _ = self.shutdown_tx.blocking_send(());
        let handle = self.join_handle.lock().unwrap().take();
        if let Some(handle) = handle {
            let _ = handle.join();
        }
    }

    /// Process notify events: queue on receive, flush on timer.
    async fn event_loop(
        mut event_rx: mpsc::Receiver<Event>,
        mut shutdown_rx: mpsc::Receiver<()>,
        mut workspace: WorkspaceState,
        db: Arc<DbWriter>,
        security_rules: Arc<std::sync::RwLock<Arc<SecurityRuleSet>>>,
        trace_state: Arc<std::sync::Mutex<TraceState>>,
    ) {
        let mut queue: Vec<QueuedEvent> = Vec::new();
        let mut dropped: u64 = 0;
        // A fixed-cadence interval, not a `sleep` recreated each iteration: the
        // select! restarts on every event, so a fresh per-iteration sleep would
        // have its deadline reset by each arrival and never fire under a
        // sustained event stream, starving the flush until MAX_QUEUE_SIZE drops
        // events. `interval` ticks on wall-cadence regardless of arrivals.
        let mut flush_ticker = tokio::time::interval(Duration::from_millis(FLUSH_INTERVAL_MS));
        flush_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => {
                    // Drain callbacks already accepted by the monitor, then
                    // reconcile the actual workspace state. PollWatcher may
                    // not have completed its next 500ms scan when shutdown
                    // starts, so queue drainage alone is not a visibility
                    // barrier for the latest guest writes.
                    Self::flush(&mut queue, &mut dropped, &mut workspace.snapshot, &db, &security_rules, &trace_state).await;
                    let current = workspace_snapshot(&workspace.watch_dir, &workspace.strip_prefix);
                    queue.extend(reconciliation_events(&workspace.snapshot, &current));
                    Self::flush(&mut queue, &mut dropped, &mut workspace.snapshot, &db, &security_rules, &trace_state).await;
                    debug!("host fs-monitor stopped");
                    break;
                }
                event = event_rx.recv() => {
                    let Some(event) = event else {
                        Self::flush(&mut queue, &mut dropped, &mut workspace.snapshot, &db, &security_rules, &trace_state).await;
                        let current = workspace_snapshot(&workspace.watch_dir, &workspace.strip_prefix);
                        queue.extend(reconciliation_events(&workspace.snapshot, &current));
                        Self::flush(&mut queue, &mut dropped, &mut workspace.snapshot, &db, &security_rules, &trace_state).await;
                        debug!("host fs-monitor channel closed");
                        break;
                    };
                    let Some(action) = event_to_action(&event.kind) else { continue };

                    for path in &event.paths {
                        let rel = path
                            .strip_prefix(&workspace.strip_prefix)
                            .unwrap_or(path)
                            .to_string_lossy()
                            .to_string();
                        if rel.is_empty() {
                            continue;
                        }
                        if queue.len() >= MAX_QUEUE_SIZE {
                            dropped += 1;
                        } else {
                            queue.push(QueuedEvent { path: rel, fs_path: path.clone(), action });
                        }
                    }
                }
                _ = flush_ticker.tick() => {
                    Self::flush(&mut queue, &mut dropped, &mut workspace.snapshot, &db, &security_rules, &trace_state).await;
                }
            }
        }
    }

    /// Drain the queue, coalesce same-type events per path, emit all.
    ///
    /// For each path, consecutive events of the same action type are coalesced
    /// into one. Different action types on the same path emit separately
    /// (e.g., create then delete = two emitted events).
    async fn flush(
        queue: &mut Vec<QueuedEvent>,
        dropped: &mut u64,
        snapshot: &mut HashMap<String, SnapshotEntry>,
        db: &DbWriter,
        security_rules: &Arc<std::sync::RwLock<Arc<SecurityRuleSet>>>,
        trace_state: &Arc<std::sync::Mutex<TraceState>>,
    ) {
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
        let mut pending: HashMap<String, (FileAction, PathBuf)> = HashMap::new();
        let mut emitted: u64 = 0;

        for event in batch {
            match pending.get(&event.path) {
                Some((existing, _)) if *existing == event.action => {
                    // Same path, same action -- coalesce (skip)
                }
                Some(_) => {
                    // Same path, different action -- emit the old one first
                    let (old_action, old_fs_path) = pending
                        .insert(event.path.clone(), (event.action, event.fs_path.clone()))
                        .unwrap();
                    let kind = resolve_kind(snapshot, &event.path, &old_fs_path, old_action);
                    Self::emit(
                        db,
                        security_rules,
                        trace_state,
                        &event.path,
                        &old_fs_path,
                        old_action,
                        kind,
                    )
                    .await;
                    Self::update_snapshot(snapshot, &event.path, &old_fs_path, old_action);
                    emitted += 1;
                }
                None => {
                    pending.insert(event.path, (event.action, event.fs_path));
                }
            }
        }

        // Emit all remaining pending entries
        for (path, (action, fs_path)) in pending {
            let kind = resolve_kind(snapshot, &path, &fs_path, action);
            Self::emit(db, security_rules, trace_state, &path, &fs_path, action, kind).await;
            Self::update_snapshot(snapshot, &path, &fs_path, action);
            emitted += 1;
        }

        if emitted > 0 {
            debug!(raw = raw_count, emitted, "fs-monitor flush");
        }
    }

    fn update_snapshot(snapshot: &mut HashMap<String, SnapshotEntry>, path: &str, fs_path: &Path, action: FileAction) {
        if action == FileAction::Deleted {
            snapshot.remove(path);
        } else if let Some(entry) = snapshot_entry(fs_path) {
            snapshot.insert(path.to_string(), entry);
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn emit(
        db: &DbWriter,
        security_rules: &Arc<std::sync::RwLock<Arc<SecurityRuleSet>>>,
        trace_state: &Arc<std::sync::Mutex<TraceState>>,
        path: &str,
        fs_path: &Path,
        action: FileAction,
        kind: FileKind,
    ) {
        // A directory's inode size says nothing about what changed inside it,
        // and reporting it as the event's size is what made `mkdir` read like
        // a small file write.
        let size = if action != FileAction::Deleted && kind != FileKind::Dir {
            std::fs::metadata(fs_path).ok().map(|m| m.len())
        } else {
            None
        };
        // Recover a poisoned rules lock rather than panic: an unwrap here kills
        // the monitor thread, silently ending all fs-event recording. Matches
        // the trace_state recovery just below.
        let rules = security_rules.read().unwrap_or_else(|e| e.into_inner()).clone();
        let trace_id = {
            let state = trace_state.lock().unwrap_or_else(|e| e.into_inner());
            state
                .lookup_file_path(path)
                .or_else(capsem_foundation::telemetry::ambient_capsem_trace_id)
        };
        let credential_ref = Self::broker_env_file_credentials(db, &rules, path, fs_path, action).await;
        crate::security_engine::emit_file_security_write_and_rules(
            db,
            &rules,
            FileEvent {
                event_id: None,
                timestamp: SystemTime::now(),
                action,
                path: path.to_string(),
                size,
                kind,
                trace_id,
                credential_ref,
            },
        )
        .await;
    }

    async fn broker_env_file_credentials(
        db: &DbWriter,
        rules: &SecurityRuleSet,
        path: &str,
        fs_path: &Path,
        action: FileAction,
    ) -> Option<String> {
        if action == FileAction::Deleted || !is_env_candidate(path) {
            return None;
        }
        let metadata = std::fs::metadata(fs_path).ok()?;
        if !metadata.is_file() || metadata.len() > 1024 * 1024 {
            return None;
        }
        let content = std::fs::read_to_string(fs_path).ok()?;
        let observations = parse_env_credentials(path, &content);
        if observations.is_empty() {
            return None;
        }
        broker_and_log_observations(db, rules, observations).await
    }
}

fn is_env_candidate(path: &str) -> bool {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == ".env" || name.starts_with(".env."))
}

#[cfg(test)]
mod tests;
