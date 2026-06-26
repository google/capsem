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

use capsem_logger::{DbWriter, FileAction, FileEvent};

use crate::credential_broker::{broker_and_log_observations, parse_env_credentials};
use crate::net::ai_traffic::TraceState;
use crate::net::policy_config::SecurityRuleSet;

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
    fs_path: PathBuf,
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
        let (event_tx, event_rx) = mpsc::channel::<Event>(1024);
        let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>(1);

        let config = Config::default().with_poll_interval(Duration::from_millis(POLL_INTERVAL_MS));
        let mut watcher = PollWatcher::new(
            move |res: Result<Event, _>| {
                if let Ok(event) = res {
                    let _ = event_tx.blocking_send(event);
                }
            },
            config,
        )?;

        watcher.watch(&watch_dir, RecursiveMode::Recursive)?;
        info!(dir = %watch_dir.display(), poll_ms = POLL_INTERVAL_MS,
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
                    strip_prefix,
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
        strip_prefix: PathBuf,
        db: Arc<DbWriter>,
        security_rules: Arc<std::sync::RwLock<Arc<SecurityRuleSet>>>,
        trace_state: Arc<std::sync::Mutex<TraceState>>,
    ) {
        let mut queue: Vec<QueuedEvent> = Vec::new();
        let mut dropped: u64 = 0;
        let flush_interval = Duration::from_millis(FLUSH_INTERVAL_MS);

        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => {
                    // Final flush
                    Self::flush(&mut queue, &mut dropped, &db, &security_rules, &trace_state).await;
                    debug!("host fs-monitor stopped");
                    break;
                }
                event = event_rx.recv() => {
                    let Some(event) = event else {
                        Self::flush(&mut queue, &mut dropped, &db, &security_rules, &trace_state).await;
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
                            queue.push(QueuedEvent { path: rel, fs_path: path.clone(), action });
                        }
                    }
                }
                _ = tokio::time::sleep(flush_interval) => {
                    Self::flush(&mut queue, &mut dropped, &db, &security_rules, &trace_state).await;
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
        db: &DbWriter,
        security_rules: &Arc<std::sync::RwLock<Arc<SecurityRuleSet>>>,
        trace_state: &Arc<std::sync::Mutex<TraceState>>,
    ) {
        if queue.is_empty() && *dropped == 0 {
            return;
        }

        if *dropped > 0 {
            warn!(
                count = *dropped,
                "fs-monitor queue overflow, events dropped"
            );
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
                    Self::emit(
                        db,
                        security_rules,
                        trace_state,
                        &event.path,
                        &old_fs_path,
                        old_action,
                    )
                    .await;
                    emitted += 1;
                }
                None => {
                    pending.insert(event.path, (event.action, event.fs_path));
                }
            }
        }

        // Emit all remaining pending entries
        for (path, (action, fs_path)) in pending {
            Self::emit(db, security_rules, trace_state, &path, &fs_path, action).await;
            emitted += 1;
        }

        if emitted > 0 {
            debug!(raw = raw_count, emitted, "fs-monitor flush");
        }
    }

    async fn emit(
        db: &DbWriter,
        security_rules: &Arc<std::sync::RwLock<Arc<SecurityRuleSet>>>,
        trace_state: &Arc<std::sync::Mutex<TraceState>>,
        path: &str,
        fs_path: &Path,
        action: FileAction,
    ) {
        let size = if action != FileAction::Deleted {
            std::fs::metadata(fs_path).ok().map(|m| m.len())
        } else {
            None
        };
        let rules = security_rules.read().unwrap().clone();
        let trace_id = {
            let state = trace_state.lock().unwrap_or_else(|e| e.into_inner());
            state
                .lookup_file_path(path)
                .or_else(crate::telemetry::ambient_capsem_trace_id)
        };
        let credential_ref =
            Self::broker_env_file_credentials(db, &rules, path, fs_path, action).await;
        crate::security_engine::emit_file_security_write_and_rules(
            db,
            &rules,
            FileEvent {
                event_id: None,
                timestamp: SystemTime::now(),
                action,
                path: path.to_string(),
                size,
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

    /// Signal the monitor to stop.
    pub async fn stop(&self) {
        let _ = self.shutdown_tx.send(()).await;
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
