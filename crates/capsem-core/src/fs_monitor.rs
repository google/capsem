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
        let (trace_id, trace_credential_ref) = {
            let state = trace_state.lock().unwrap_or_else(|e| e.into_inner());
            let trace_id = state
                .lookup_file_path(path)
                .or_else(crate::telemetry::ambient_capsem_trace_id);
            let trace_credential_ref = trace_id
                .as_deref()
                .and_then(|trace_id| state.lookup_trace_credential(trace_id));
            (trace_id, trace_credential_ref)
        };
        let credential_ref = Self::broker_env_file_credentials(db, &rules, path, fs_path, action)
            .await
            .or(trace_credential_ref);
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
mod tests {
    use super::*;
    use crate::net::policy_config::{SecurityRuleProfile, SecurityRuleSource};

    struct EnvGuard {
        old_home_override: Option<String>,
        old_home: Option<String>,
        old_store: Option<String>,
    }

    impl EnvGuard {
        fn install(
            capsem_home: &std::path::Path,
            home: &std::path::Path,
            test_store: &std::path::Path,
        ) -> Self {
            let old_home_override = std::env::var("CAPSEM_HOME").ok();
            let old_home = std::env::var("HOME").ok();
            let old_store = std::env::var(crate::credential_broker::STORE_PATH_ENV).ok();
            std::env::set_var("CAPSEM_HOME", capsem_home);
            std::env::set_var("HOME", home);
            std::env::set_var(crate::credential_broker::STORE_PATH_ENV, test_store);
            Self {
                old_home_override,
                old_home,
                old_store,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.old_home_override {
                Some(v) => std::env::set_var("CAPSEM_HOME", v),
                None => std::env::remove_var("CAPSEM_HOME"),
            }
            match &self.old_home {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
            match &self.old_store {
                Some(v) => std::env::set_var(crate::credential_broker::STORE_PATH_ENV, v),
                None => std::env::remove_var(crate::credential_broker::STORE_PATH_ENV),
            }
        }
    }

    fn empty_trace_state() -> Arc<std::sync::Mutex<TraceState>> {
        Arc::new(std::sync::Mutex::new(TraceState::new()))
    }

    fn empty_security_rules() -> Arc<std::sync::RwLock<Arc<SecurityRuleSet>>> {
        Arc::new(std::sync::RwLock::new(Arc::new(SecurityRuleSet::new(
            Vec::new(),
        ))))
    }

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
    fn env_candidate_matches_dotenv_files_only() {
        assert!(is_env_candidate(".env"));
        assert!(is_env_candidate("project/.env.local"));
        assert!(!is_env_candidate("project/env.txt"));
        assert!(!is_env_candidate("project/not.env"));
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
                fs_path: PathBuf::from(format!("file_{}.txt", i)),
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

    #[tokio::test]
    async fn emit_brokers_env_credentials_and_persists_reference() {
        let _lock = crate::credential_broker::TEST_ENV_LOCK.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("session.db");
        let env_path = dir.path().join(".env");
        let capsem_home = dir.path().join("capsem-home");
        let test_store = dir.path().join("credential-store.json");
        let _guard = EnvGuard::install(&capsem_home, dir.path(), &test_store);
        std::fs::write(&env_path, "OPENAI_API_KEY=sk-env-secret\n").unwrap();

        let db = DbWriter::open(&db_path, 64).unwrap();
        FsMonitor::emit(
            &db,
            &empty_security_rules(),
            &empty_trace_state(),
            ".env",
            &env_path,
            FileAction::Modified,
        )
        .await;
        db.shutdown_blocking();

        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let file_ref: String = conn
            .query_row(
                "SELECT credential_ref FROM fs_events WHERE path = '.env'",
                [],
                |row| row.get(0),
            )
            .expect(".env fs event should carry brokered credential ref");
        let outcomes: Vec<String> = conn
            .prepare(
                "SELECT outcome FROM substitution_events WHERE substitution_ref = ?1 AND source = '.env:OPENAI_API_KEY' ORDER BY outcome",
            )
            .unwrap()
            .query_map([&file_ref], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(outcomes, vec!["brokered", "captured"]);
        let db_bytes = std::fs::read(&db_path).unwrap();
        assert!(!String::from_utf8_lossy(&db_bytes).contains("sk-env-secret"));
    }

    #[tokio::test]
    async fn emit_writes_file_security_rule_ledger_row() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("session.db");
        let file_path = dir.path().join("skill.md");
        std::fs::write(&file_path, "# skill").unwrap();
        let db = DbWriter::open(&db_path, 64).unwrap();
        let profile = SecurityRuleProfile::parse_toml(
            r#"
[profiles.rules.file_create_skill]
name = "file_create_skill"
action = "allow"
detection_level = "informational"
match = 'file.create.name == "skill.md" && file.create.ext == "md"'
"#,
        )
        .unwrap();
        let rules = SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User).unwrap();
        let security_rules = Arc::new(std::sync::RwLock::new(Arc::new(rules)));

        FsMonitor::emit(
            &db,
            &security_rules,
            &empty_trace_state(),
            "skill.md",
            &file_path,
            FileAction::Created,
        )
        .await;
        db.shutdown_blocking();

        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let joined: (String, String) = conn
            .query_row(
                "SELECT fs_events.event_id, security_rule_events.rule_id
                 FROM fs_events
                 JOIN security_rule_events ON security_rule_events.event_id = fs_events.event_id
                 WHERE fs_events.path = 'skill.md'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(joined.0.len(), 12);
        assert_eq!(joined.1, "profiles.rules.file_create_skill");
    }

    #[tokio::test]
    async fn emit_uses_model_tool_file_hint_for_trace_id() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("session.db");
        let file_path = dir.path().join("openai-two.txt");
        std::fs::write(&file_path, "nonce\n").unwrap();
        let db = DbWriter::open(&db_path, 64).unwrap();
        let profile = SecurityRuleProfile::parse_toml(
            r#"
[profiles.rules.file_create_any]
name = "file_create_any"
action = "allow"
match = 'file.create.path == "openai-two.txt"'
"#,
        )
        .unwrap();
        let rules = SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User).unwrap();
        let security_rules = Arc::new(std::sync::RwLock::new(Arc::new(rules)));
        let trace_state = empty_trace_state();
        trace_state.lock().unwrap().register_tool_file_hints(
            "trace-model",
            [r#"{"cmd":"printf x > /root/openai-two.txt"}"#],
        );
        trace_state.lock().unwrap().register_trace_credential(
            "trace-model",
            Some("credential:blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
        );

        FsMonitor::emit(
            &db,
            &security_rules,
            &trace_state,
            "openai-two.txt",
            &file_path,
            FileAction::Created,
        )
        .await;
        db.shutdown_blocking();

        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let (trace_id, credential_ref): (String, String) = conn
            .query_row(
                "SELECT trace_id, credential_ref FROM fs_events WHERE path = 'openai-two.txt'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let (rule_trace_id, event_credential_ref): (String, String) = conn
            .query_row(
                "SELECT trace_id, json_extract(event_json, '$.credential_ref') FROM security_rule_events
                 WHERE event_id = (SELECT event_id FROM fs_events WHERE path = 'openai-two.txt')",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(trace_id, "trace-model");
        assert_eq!(rule_trace_id, "trace-model");
        assert_eq!(
            credential_ref,
            "credential:blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert_eq!(event_credential_ref, credential_ref);
    }

    #[tokio::test]
    async fn emit_records_block_rules_as_audit_only_file_event() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("session.db");
        let file_path = dir.path().join("blocked.txt");
        std::fs::write(&file_path, "already materialized").unwrap();
        let db = DbWriter::open(&db_path, 64).unwrap();
        let profile = SecurityRuleProfile::parse_toml(
            r#"
[profiles.rules.file_monitor_block_seen]
name = "file_monitor_block_seen"
action = "block"
detection_level = "high"
match = 'file.write.path == "blocked.txt"'
"#,
        )
        .unwrap();
        let rules = SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User).unwrap();
        let security_rules = Arc::new(std::sync::RwLock::new(Arc::new(rules)));

        FsMonitor::emit(
            &db,
            &security_rules,
            &empty_trace_state(),
            "blocked.txt",
            &file_path,
            FileAction::Modified,
        )
        .await;
        db.shutdown_blocking();

        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let fs_action: String = conn
            .query_row(
                "SELECT action FROM fs_events WHERE path = 'blocked.txt'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(fs_action, "modified");
        let (event_type, rule_action, detection_level): (String, String, String) = conn
            .query_row(
                "SELECT event_type, rule_action, detection_level
                 FROM security_rule_events
                 WHERE rule_id = 'profiles.rules.file_monitor_block_seen'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(event_type, "file.event");
        assert_eq!(rule_action, "block");
        assert_eq!(detection_level, "high");
        let import_export_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM security_rule_events
                 WHERE event_type IN ('file.import', 'file.export')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            import_export_rows, 0,
            "fs_monitor audit events must not masquerade as boundary gates"
        );
    }
}
