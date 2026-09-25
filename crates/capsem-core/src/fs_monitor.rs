//! Host-based file monitor for VirtioFS overlay directories.
//!
//! Apple VZ VirtioFS writes bypass FSEvents, so the workspace has to be
//! polled. The monitor owns that poll loop rather than delegating it: it walks
//! the tree descriptor-relative, following nothing, diffs the walk against the
//! previous one, and emits the difference.
//!
//! Owning the loop is a security property, not a preference. The `notify`
//! crate's polling watcher walks with link-following hardcoded on and ignores
//! the configuration that claims to turn it off, so one guest `ln -s /
//! workspace/evil` turned every scan into a walk of the host's entire
//! filesystem. Nothing here follows a link: a symlink is recorded as a symlink
//! and never descended, and the one place that reads bytes (`.env` credential
//! brokering) opens with `O_NOFOLLOW` and refuses anything that is not a
//! regular file. The walk and that open both start from a workspace descriptor
//! opened once, from the host-owned session directory: the workspace is an
//! entry of the guest's share, and a guest that swaps it for a link to a host
//! directory must not get that directory walked into its ledger or its `.env`
//! files brokered. `tests/citadel/test_fs_monitor_has_no_exclusions.py` refuses
//! that watcher and that setting by name, which is why this paragraph spells
//! neither.
//!
//! Owning the loop also means the cadence tracks the real cost: each scan
//! times itself and sets the next sleep, so a workspace that starts empty and
//! ephemeral and later grows to a 100k-file install re-adapts on the next
//! cycle instead of keeping the interval it was born with.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use capsem_foundation::unix::contained::{ContainedDir, ContainedEntry, ContainedOpenOptions, EntryKind};
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

/// The one place a listed entry becomes a ledger `kind`. A symlink is a
/// symlink, never the thing it points at.
fn kind_of(entry: &ContainedEntry) -> FileKind {
    match entry.kind {
        EntryKind::Directory => FileKind::Dir,
        EntryKind::File => FileKind::File,
        EntryKind::Other if entry.is_symlink => FileKind::Symlink,
        EntryKind::Other => FileKind::Other,
    }
}

/// One emitted change. Only the workspace-relative path is kept: at 100k queued events the `PathBuf` was half the memory and
/// every byte of it was already in `path`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct QueuedEvent {
    path: String,
    action: FileAction,
    kind: FileKind,
    size: Option<u64>,
}

/// What one scan saw of one path, and the whole of what "unchanged" means.
///
/// `mtime` alone is guest-controlled and cheap to forge: rewrite a file in
/// place to the same length, then `touch -r` it back, and a monitor comparing
/// (kind, len, mtime) sees nothing. `ctime` moves on any inode update and
/// cannot be set by `utimes`, and `ino` catches a replacement that reuses the
/// path. Both come out of the stat the walk already did.
#[derive(Clone, Debug, PartialEq, Eq)]
struct SnapshotEntry {
    kind: FileKind,
    len: u64,
    modified: Option<(u64, u32)>,
    changed: (i64, i64),
    ino: u64,
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
fn workspace_snapshot(workspace: &ContainedDir) -> HashMap<String, SnapshotEntry> {
    // Every entry, with nothing pruned: a walk that skips a directory is a
    // ledger that lies about it.
    let mut snapshot = HashMap::new();
    let mut pending = match workspace.try_clone() {
        Ok(root) => vec![(root, String::new())],
        Err(_) => Vec::new(),
    };
    while let Some((dir, prefix)) = pending.pop() {
        let _ = dir.visit_entries(|entry| {
            // A non-UTF8 filename is recorded lossily rather than skipped: a
            // name the ledger cannot spell exactly is still a change that
            // happened, and dropping it would be one more way to write a file
            // the record does not mention.
            let name = entry.name.to_string_lossy();
            let rel = if prefix.is_empty() {
                name.into_owned()
            } else {
                format!("{prefix}/{name}")
            };
            if entry.kind == EntryKind::Directory {
                // Refuses a link swapped in since the listing's own lstat.
                if let Ok(child) = dir.descend(&entry.name) {
                    pending.push((child, rel.clone()));
                }
            }
            let (seconds, nanos) = entry.identity.mtime;
            snapshot.insert(
                rel,
                SnapshotEntry {
                    kind: kind_of(&entry),
                    len: entry.identity.size,
                    modified: u64::try_from(seconds).ok().zip(u32::try_from(nanos).ok()),
                    changed: entry.identity.ctime,
                    ino: entry.identity.ino,
                },
            );
            Ok(true)
        });
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

/// Bound one scan's emission and rewind the baseline for what it defers.
///
/// Truncating used to be permanent: the overflow was logged, `current` became
/// the baseline, and the difference was never derived again. That is a hole an
/// attacker can steer, because events come out ordered by path -- 100k files
/// named `!...` push `.git/hooks/pre-commit` out of the window, and nothing in
/// the ledger says so.
///
/// Rewinding each deferred path to what `previous` held (or removing it, if
/// `previous` had never seen it) makes the loss temporary: the next scan
/// computes the same difference for that path and emits it. The order is a
/// delay, not a filter.
///
/// What a delay does cost is resolution, and only in one direction: a deferred
/// deletion whose path is re-created before the next scan surfaces as a
/// `Modified`, because that is what the two snapshots then differ by. Snapshot
/// polling has that blind spot at any interval -- a delete-and-recreate inside
/// one window reads the same way -- and deferral widens the window by one
/// scan for the paths it holds back. The `overflow` row says which windows
/// those were.
fn defer_overflow(
    batch: &mut Vec<QueuedEvent>,
    previous: &HashMap<String, SnapshotEntry>,
    current: &mut HashMap<String, SnapshotEntry>,
    max_batch: usize,
) -> usize {
    if batch.len() <= max_batch {
        return 0;
    }
    let deferred = batch.split_off(max_batch);
    for event in &deferred {
        match previous.get(&event.path) {
            Some(entry) => current.insert(event.path.clone(), entry.clone()),
            None => current.remove(&event.path),
        };
    }
    deferred.len()
}

/// The marker row for a window the monitor could not record in full.
fn overflow_event(deferred: usize) -> QueuedEvent {
    QueuedEvent {
        path: String::new(),
        action: FileAction::Overflow,
        kind: FileKind::Other,
        size: Some(deferred as u64),
    }
}

/// Everything the scan loop needs about the tree it watches.
struct ScanConfig {
    workspace: ContainedDir,
    /// Cadence for the next scan; re-derived from each scan's own cost.
    interval: Duration,
    /// Events emitted from one scan before the rest is deferred to the next.
    /// Only the tests lower it; production is `MAX_QUEUE_SIZE`.
    max_batch: usize,
}

/// What every emission needs, bundled so the signature stays readable.
struct EmitContext<'a> {
    db: &'a DbWriter,
    security_rules: &'a Arc<std::sync::RwLock<Arc<SecurityRuleSet>>>,
    trace_state: &'a Arc<std::sync::Mutex<TraceState>>,
    /// The watched tree; recorded paths are relative to it.
    workspace: &'a ContainedDir,
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
    ///
    /// The wait it imposes is bounded: at most two scans and two emissions.
    /// A shutdown noticed mid-emission costs one extra cycle, to reconcile
    /// what was written while that batch was persisting, and never more.
    join_handle: std::sync::Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl FsMonitor {
    /// Start monitoring `workspace` and writing events, with paths relative
    /// to it, to `db`. Open it with `crate::session::open_workspace`.
    pub fn start(
        workspace: ContainedDir,
        db: Arc<DbWriter>,
        security_rules: Arc<std::sync::RwLock<Arc<SecurityRuleSet>>>,
        trace_state: Arc<std::sync::Mutex<TraceState>>,
    ) -> anyhow::Result<Self> {
        // The baseline walk is the same walk each scan performs, so it is also
        // the first measurement of what a scan costs.
        let scan_started = Instant::now();
        let snapshot = workspace_snapshot(&workspace);
        let scan_duration = scan_started.elapsed();
        let poll_interval = poll_interval_for_scan(scan_duration);
        let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>(1);

        info!(dir = %workspace.path().display(), entries = snapshot.len(), scan_ms = scan_duration.as_millis(),
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
                    ScanConfig {
                        workspace,
                        interval: poll_interval,
                        max_batch: MAX_QUEUE_SIZE,
                    },
                    snapshot,
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
    async fn scan_loop(
        mut shutdown_rx: mpsc::Receiver<()>,
        config: ScanConfig,
        mut snapshot: HashMap<String, SnapshotEntry>,
        db: Arc<DbWriter>,
        security_rules: Arc<std::sync::RwLock<Arc<SecurityRuleSet>>>,
        trace_state: Arc<std::sync::Mutex<TraceState>>,
    ) {
        let ctx = EmitContext {
            db: &db,
            security_rules: &security_rules,
            trace_state: &trace_state,
            workspace: &config.workspace,
        };
        let mut interval = config.interval;
        let mut stopping = false;

        loop {
            if !stopping {
                stopping = tokio::select! {
                    _ = shutdown_rx.recv() => true,
                    _ = tokio::time::sleep(interval) => false,
                };
            }

            let scan_started = Instant::now();
            let mut current = workspace_snapshot(&config.workspace);
            let scan_duration = scan_started.elapsed();
            let mut batch = reconciliation_events(&snapshot, &current);
            let raw = batch.len();
            let deferred = defer_overflow(&mut batch, &snapshot, &mut current, config.max_batch);
            if deferred > 0 {
                warn!(
                    count = deferred,
                    "fs-monitor scan overflow, events deferred to next scan"
                );
                // The marker goes in the same batch as the window it describes,
                // so the gap is read in place rather than inferred from a log.
                batch.push(overflow_event(deferred));
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
    /// scan's own lstat, and every component is opened `O_NOFOLLOW` from the
    /// workspace descriptor, regular-file-only for the last, so a link swapped
    /// in after the scan -- at any depth -- is refused too.
    async fn broker_env_file_credentials(
        ctx: &EmitContext<'_>,
        rules: &SecurityRuleSet,
        event: &QueuedEvent,
    ) -> Option<String> {
        if event.action == FileAction::Deleted || event.kind != FileKind::File || !is_env_candidate(&event.path) {
            return None;
        }
        let path = Path::new(&event.path);
        let file = ctx
            .workspace
            .walk(path.parent()?)
            .ok()?
            .open_file(path.file_name()?, ContainedOpenOptions::read_only())
            .ok()?;
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
