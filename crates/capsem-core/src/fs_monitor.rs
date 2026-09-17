//! Host-based file monitor for VirtioFS overlay directories.
//!
//! Apple VZ VirtioFS writes bypass FSEvents, so the workspace has to be
//! polled. The monitor owns that poll loop rather than delegating it: it walks
//! the tree with `walkdir` and `follow_links(false)`, diffs the walk against
//! the previous one, and emits the difference.
//!
//! Owning the loop is a security property, not a preference. `notify`'s
//! `PollWatcher` walks with `follow_links(true)` hardcoded and ignores
//! `configure`, so one guest `ln -s / workspace/evil` turned every scan into a
//! walk of the host's entire filesystem. Nothing here follows a link: a
//! symlink is recorded as a symlink and never descended, and the one place
//! that reads bytes (`.env` credential brokering) opens with `O_NOFOLLOW` and
//! refuses anything that is not a regular file.
//!
//! Owning the loop also means the cadence tracks the real cost: each scan
//! times itself and sets the next sleep, so a workspace that starts empty and
//! ephemeral and later grows to a 100k-file install re-adapts on the next
//! cycle instead of keeping the interval it was born with.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use capsem_foundation::unix::fs as unix_fs;
use capsem_logger::{DbWriter, FileAction, FileEvent, FileKind};

use crate::credential_broker::{broker_and_log_observations, parse_env_credentials};
use crate::net::ai_traffic::TraceState;
use crate::net::policy_config::SecurityRuleSet;

/// Floor and ceiling for the rescan cadence (ms).
const POLL_INTERVAL_MIN_MS: u64 = 500;
const POLL_INTERVAL_MAX_MS: u64 = 10_000;

/// Scans per interval. One scan stats every entry of the workspace, and the
/// monitor watches every path -- including `node_modules` and the build output
/// directory, which is the whole point -- so the cost is real and proportional
/// to the tree. Spending at most a tenth of the wall clock on it keeps the
/// monitor from competing with the workload it is watching, at any tree size.
const POLL_SCAN_DUTY_CYCLE: u32 = 10;

/// Poll cadence for a tree whose full stat walk took `scan`.
///
/// Ten scans per interval, floored at 500ms so a small workspace still reacts
/// promptly, capped at 10s so a very large one is still watched. Cost is
/// answered here, never by deciding in advance which paths are not worth
/// recording: `.git/hooks`, `node_modules` and the build output directory are
/// precisely where a compromise persists.
fn poll_interval_for_scan(scan: Duration) -> Duration {
    (scan * POLL_SCAN_DUTY_CYCLE).clamp(
        Duration::from_millis(POLL_INTERVAL_MIN_MS),
        Duration::from_millis(POLL_INTERVAL_MAX_MS),
    )
}

/// Maximum number of events emitted from a single scan.
///
/// A 100k-file install between two scans must not lose events. An event is a
/// path string plus two small enums and an `Option<u64>`, so a full queue is
/// tens of megabytes transient, and only for as long as it takes to write.
const MAX_QUEUE_SIZE: usize = 100_000;

/// Events written between two checks of the shutdown signal.
const EMIT_CHUNK: usize = 1_000;

/// Largest `.env` the credential broker will read.
const MAX_ENV_BYTES: u64 = 1024 * 1024;

/// The one place a `FileType` becomes a ledger `kind`.
///
/// Order matters: a symlink to a directory reports `is_dir()` through a
/// following stat, and calling it a directory is exactly the confusion that
/// lets a link pass for the thing it points at.
fn kind_of(file_type: std::fs::FileType) -> FileKind {
    if file_type.is_symlink() {
        FileKind::Symlink
    } else if file_type.is_dir() {
        FileKind::Dir
    } else if file_type.is_file() {
        FileKind::File
    } else {
        FileKind::Other
    }
}

/// One emitted change. The absolute path is derived from `strip_prefix` rather
/// than stored: at 100k queued events the `PathBuf` was half the memory and
/// every byte of it was already in `path`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct QueuedEvent {
    path: String,
    action: FileAction,
    kind: FileKind,
    size: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SnapshotEntry {
    kind: FileKind,
    len: u64,
    modified: Option<(u64, u32)>,
}

impl SnapshotEntry {
    /// A directory's inode size says nothing about what changed inside it, and
    /// a symlink's is the length of its target string; reporting either as the
    /// event's size is what made `mkdir` read like a small file write.
    fn size(&self) -> Option<u64> {
        matches!(self.kind, FileKind::File | FileKind::Other).then_some(self.len)
    }
}

/// Walk the workspace, stat-ing each entry exactly once and following nothing.
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
        // `follow_links(false)` makes this the entry's own lstat, and it is
        // the only stat any part of the monitor performs for this path.
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let modified = metadata
            .modified()
            .ok()
            .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|value| (value.as_secs(), value.subsec_nanos()));
        snapshot.insert(
            rel,
            SnapshotEntry {
                kind: kind_of(metadata.file_type()),
                len: metadata.len(),
                modified,
            },
        );
    }
    snapshot
}

/// The difference between two scans, as events.
///
/// A path created and deleted entirely between two scans leaves no trace in
/// either snapshot and so produces nothing at all -- it is not recorded with a
/// guessed kind, because the monitor never saw it and inventing `file` for it
/// would be a claim it cannot support.
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
                action: FileAction::Created,
                kind: entry.kind,
                size: entry.size(),
            }),
            (Some(entry), None) => Some(QueuedEvent {
                path,
                action: FileAction::Deleted,
                // The last snapshot is the only remaining witness to what
                // disappeared; there is nothing left to stat.
                kind: entry.kind,
                size: None,
            }),
            (Some(before), Some(after)) if before != after => Some(QueuedEvent {
                path,
                action: FileAction::Modified,
                kind: after.kind,
                size: after.size(),
            }),
            _ => None,
        })
        .collect()
}

/// Bound one scan's emission, reporting how many events were dropped.
fn cap_batch(batch: &mut Vec<QueuedEvent>) -> usize {
    let dropped = batch.len().saturating_sub(MAX_QUEUE_SIZE);
    batch.truncate(MAX_QUEUE_SIZE);
    dropped
}

/// What every emission needs, bundled so the signature stays readable.
struct EmitContext<'a> {
    db: &'a DbWriter,
    security_rules: &'a Arc<std::sync::RwLock<Arc<SecurityRuleSet>>>,
    trace_state: &'a Arc<std::sync::Mutex<TraceState>>,
    /// Prefix removed from absolute paths when recording, and therefore the
    /// prefix that turns a recorded path back into one on disk.
    strip_prefix: &'a Path,
}

/// Host-side file system monitor.
///
/// Watches the VirtioFS workspace directory by stat-based polling, on a
/// cadence it derives from its own scan cost. See the module doc for why the
/// loop is owned here rather than taken from `notify`.
pub struct FsMonitor {
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
        // The baseline walk is the same walk each scan performs, so it is also
        // the first measurement of what a scan costs.
        let scan_started = Instant::now();
        let snapshot = workspace_snapshot(&watch_dir, &strip_prefix);
        let scan_duration = scan_started.elapsed();
        let poll_interval = poll_interval_for_scan(scan_duration);
        let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>(1);

        info!(dir = %watch_dir.display(), entries = snapshot.len(), scan_ms = scan_duration.as_millis(),
              poll_ms = poll_interval.as_millis(),
              "host fs-monitor started (poll mode, FSEvents unreliable for VirtioFS)");

        let join_handle = std::thread::Builder::new()
            .name("capsem-fs-monitor".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_time()
                    .build()
                    .expect("fs_monitor runtime");
                rt.block_on(Self::scan_loop(
                    shutdown_rx,
                    watch_dir,
                    strip_prefix,
                    snapshot,
                    poll_interval,
                    db,
                    security_rules,
                    trace_state,
                ));
            })
            .expect("failed to spawn fs_monitor thread");

        Ok(Self {
            shutdown_tx,
            join_handle: std::sync::Mutex::new(Some(join_handle)),
        })
    }

    /// Signal the scan loop to reconcile and exit, then block until the
    /// worker thread has run its final flush into `DbWriter`. Idempotent.
    /// Call from a blocking context (e.g. `tokio::task::spawn_blocking`).
    pub fn shutdown_and_join(&self) {
        let _ = self.shutdown_tx.blocking_send(());
        let handle = self.join_handle.lock().unwrap().take();
        if let Some(handle) = handle {
            let _ = handle.join();
        }
    }

    /// Scan, emit the difference, sleep for as long as the scan earned.
    ///
    /// Shutdown is one more scan and emission -- the same code path as any
    /// other cycle -- so the last writes before teardown land in the ledger
    /// without a second reconciliation mechanism to keep in step.
    #[allow(clippy::too_many_arguments)]
    async fn scan_loop(
        mut shutdown_rx: mpsc::Receiver<()>,
        watch_dir: PathBuf,
        strip_prefix: PathBuf,
        mut snapshot: HashMap<String, SnapshotEntry>,
        mut interval: Duration,
        db: Arc<DbWriter>,
        security_rules: Arc<std::sync::RwLock<Arc<SecurityRuleSet>>>,
        trace_state: Arc<std::sync::Mutex<TraceState>>,
    ) {
        let ctx = EmitContext {
            db: &db,
            security_rules: &security_rules,
            trace_state: &trace_state,
            strip_prefix: &strip_prefix,
        };
        let mut stopping = false;

        loop {
            if !stopping {
                stopping = tokio::select! {
                    _ = shutdown_rx.recv() => true,
                    _ = tokio::time::sleep(interval) => false,
                };
            }

            let scan_started = Instant::now();
            let current = workspace_snapshot(&watch_dir, &strip_prefix);
            let scan_duration = scan_started.elapsed();
            let mut batch = reconciliation_events(&snapshot, &current);
            let raw = batch.len();
            let dropped = cap_batch(&mut batch);
            if dropped > 0 {
                warn!(count = dropped, "fs-monitor scan overflow, events dropped");
            }
            snapshot = current;
            let saw_shutdown = Self::emit_batch(&ctx, &batch, &mut shutdown_rx).await;
            if raw > 0 {
                debug!(raw, emitted = batch.len(), "fs-monitor scan");
            }

            if stopping {
                debug!("host fs-monitor stopped");
                break;
            }
            if saw_shutdown {
                // Go round once more so anything written while that batch was
                // being persisted is reconciled before the thread exits.
                stopping = true;
                continue;
            }

            let next = poll_interval_for_scan(scan_duration);
            // Only a real change is worth a line: a workspace that grows into
            // a large install should say so, a scan that wobbles should not.
            if next > interval * 2 || next * 2 < interval {
                info!(
                    entries = snapshot.len(),
                    scan_ms = scan_duration.as_millis(),
                    poll_ms = next.as_millis(),
                    "host fs-monitor poll interval adapted to scan cost"
                );
            }
            interval = next;
        }
    }

    /// Write a scan's events, checking for shutdown between chunks.
    ///
    /// The check is a poll rather than a wait: a shutdown arriving mid-batch
    /// must be noticed promptly, but the batch is still finished, because the
    /// rows are the whole point of shutting down in order.
    async fn emit_batch(ctx: &EmitContext<'_>, batch: &[QueuedEvent], shutdown_rx: &mut mpsc::Receiver<()>) -> bool {
        let mut saw_shutdown = false;
        for chunk in batch.chunks(EMIT_CHUNK) {
            for event in chunk {
                Self::emit(ctx, event).await;
            }
            saw_shutdown |= shutdown_rx.try_recv().is_ok();
        }
        saw_shutdown
    }

    async fn emit(ctx: &EmitContext<'_>, event: &QueuedEvent) {
        // Recover a poisoned rules lock rather than panic: an unwrap here kills
        // the monitor thread, silently ending all fs-event recording. Matches
        // the trace_state recovery just below.
        let rules = ctx.security_rules.read().unwrap_or_else(|e| e.into_inner()).clone();
        let trace_id = {
            let state = ctx.trace_state.lock().unwrap_or_else(|e| e.into_inner());
            state
                .lookup_file_path(&event.path)
                .or_else(capsem_foundation::telemetry::ambient_capsem_trace_id)
        };
        let credential_ref = Self::broker_env_file_credentials(ctx, &rules, event).await;
        crate::security_engine::emit_file_security_write_and_rules(
            ctx.db,
            &rules,
            FileEvent {
                event_id: None,
                timestamp: SystemTime::now(),
                action: event.action,
                path: event.path.clone(),
                size: event.size,
                kind: event.kind,
                trace_id,
                credential_ref,
            },
        )
        .await;
    }

    /// Broker credentials found in a `.env` the guest just wrote.
    ///
    /// Two conditions guard the only place the monitor reads guest-controlled
    /// bytes, and both are about the same attack: a guest that plants `.env`
    /// as a symlink to a host file (`~/.aws/credentials`, a private key) so
    /// the host parses and stores what it points at. The kind comes from the
    /// scan's own lstat, and the open is `O_NOFOLLOW` and regular-file-only,
    /// so a link swapped in after the scan is refused too.
    async fn broker_env_file_credentials(
        ctx: &EmitContext<'_>,
        rules: &SecurityRuleSet,
        event: &QueuedEvent,
    ) -> Option<String> {
        if event.action == FileAction::Deleted || event.kind != FileKind::File || !is_env_candidate(&event.path) {
            return None;
        }
        let fs_path = ctx.strip_prefix.join(&event.path);
        let file = unix_fs::open_regular_file_no_follow(&fs_path).ok()?;
        // fstat on the open handle, so the size that is checked belongs to the
        // same file that is about to be read.
        if file.metadata().ok()?.len() > MAX_ENV_BYTES {
            return None;
        }
        let mut content = String::new();
        file.take(MAX_ENV_BYTES).read_to_string(&mut content).ok()?;
        let observations = parse_env_credentials(&event.path, &content);
        if observations.is_empty() {
            return None;
        }
        broker_and_log_observations(ctx.db, rules, observations).await
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
