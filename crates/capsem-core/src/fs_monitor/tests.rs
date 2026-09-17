use super::*;
use crate::net::policy_config::{SecurityRuleProfile, SecurityRuleSource};

struct EnvGuard {
    // Redirects CAPSEM_HOME/RUN_DIR/ASSETS_DIR together; restores on drop.
    _capsem_paths: capsem_foundation::paths::CapsemPathsGuard,
    old_home: Option<String>,
    old_store: Option<String>,
}

impl EnvGuard {
    fn install(capsem_home: &std::path::Path, home: &std::path::Path, test_store: &std::path::Path) -> Self {
        let old_home = std::env::var("HOME").ok();
        let old_store = std::env::var(crate::credential_broker::STORE_PATH_ENV).ok();
        std::env::set_var("HOME", home);
        std::env::set_var(crate::credential_broker::STORE_PATH_ENV, test_store);
        Self {
            _capsem_paths: capsem_foundation::paths::CapsemPathsGuard::redirect(capsem_home),
            old_home,
            old_store,
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
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
    Arc::new(std::sync::RwLock::new(Arc::new(SecurityRuleSet::new(Vec::new()))))
}

/// Start a monitor over a fresh workspace and hand back the workspace, the
/// DB path and the running monitor.
fn monitor_over_new_workspace(dir: &std::path::Path) -> (PathBuf, PathBuf, Arc<DbWriter>, FsMonitor) {
    let workspace = dir.join("workspace");
    if !workspace.exists() {
        std::fs::create_dir(&workspace).unwrap();
    }
    let db_path = dir.join("session.db");
    let db = Arc::new(DbWriter::open(&db_path, 64).unwrap());
    let monitor = FsMonitor::start(
        workspace.clone(),
        workspace.clone(),
        Arc::clone(&db),
        empty_security_rules(),
        empty_trace_state(),
    )
    .unwrap();
    (workspace, db_path, db, monitor)
}

/// A write to `path`, as the scan would report it.
fn env_event(path: &str, kind: FileKind) -> QueuedEvent {
    QueuedEvent {
        path: path.to_string(),
        action: FileAction::Modified,
        kind,
        size: Some(1),
    }
}

fn recorded_event(db_path: &Path, path: &str) -> Option<(String, String, Option<i64>)> {
    let conn = rusqlite::Connection::open(db_path).unwrap();
    conn.query_row(
        "SELECT action, kind, size FROM fs_events WHERE path = ?1",
        [path],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )
    .ok()
}

/// `.git/hooks/*` is how a compromised session persists. An exclusion list
/// that hid it made the ledger report a clean session for a backdoored repo.
#[test]
fn events_under_dot_git_hooks_are_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let (workspace, db_path, db, monitor) = monitor_over_new_workspace(dir.path());

    let hooks = workspace.join(".git/hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    std::fs::write(hooks.join("pre-commit"), "#!/bin/sh\nexfiltrate\n").unwrap();
    monitor.shutdown_and_join();
    db.shutdown_blocking();

    let hook = recorded_event(&db_path, ".git/hooks/pre-commit").expect("the hook write must be a ledger row");
    assert_eq!((hook.0.as_str(), hook.1.as_str()), ("created", "file"));
}

/// `node_modules/<pkg>/package.json` is how a supply-chain attack lands an
/// install script. Same finding, same rule.
#[test]
fn events_under_node_modules_are_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let (workspace, db_path, db, monitor) = monitor_over_new_workspace(dir.path());

    let package = workspace.join("node_modules/evil");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("package.json"), r#"{"scripts":{"postinstall":"sh -c x"}}"#).unwrap();
    monitor.shutdown_and_join();
    db.shutdown_blocking();

    let manifest =
        recorded_event(&db_path, "node_modules/evil/package.json").expect("the install script must be a ledger row");
    assert_eq!((manifest.0.as_str(), manifest.1.as_str()), ("created", "file"));
}

/// A directory event must say it is a directory. Recording `mkdir` as an
/// anonymous path carrying the directory inode's size told a reader nothing.
#[test]
fn mkdir_and_rmdir_are_recorded_as_dir_events() {
    let dir = tempfile::tempdir().unwrap();
    let (workspace, db_path, db, monitor) = monitor_over_new_workspace(dir.path());
    std::fs::create_dir(workspace.join("payload")).unwrap();
    monitor.shutdown_and_join();

    // A second monitor starts with the directory in its snapshot, so the
    // removal resolves its kind from that snapshot rather than from a path
    // that no longer exists.
    let monitor = FsMonitor::start(
        workspace.clone(),
        workspace.clone(),
        Arc::clone(&db),
        empty_security_rules(),
        empty_trace_state(),
    )
    .unwrap();
    std::fs::remove_dir(workspace.join("payload")).unwrap();
    monitor.shutdown_and_join();
    db.shutdown_blocking();

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let mut stmt = conn
        .prepare("SELECT action, kind, size FROM fs_events WHERE path = 'payload' ORDER BY id")
        .unwrap();
    let rows: Vec<(String, String, Option<i64>)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        rows,
        vec![
            ("created".to_string(), "dir".to_string(), None),
            ("deleted".to_string(), "dir".to_string(), None),
        ]
    );
}

/// Watching every path costs a full stat walk per scan, so the scan pays for
/// itself: the interval is ten scans long, floored and capped.
#[test]
fn poll_interval_scales_with_scan_cost() {
    assert_eq!(
        poll_interval_for_scan(Duration::from_millis(1)),
        Duration::from_millis(500)
    );
    assert_eq!(
        poll_interval_for_scan(Duration::from_millis(200)),
        Duration::from_secs(2)
    );
    assert_eq!(poll_interval_for_scan(Duration::from_secs(5)), Duration::from_secs(10));
}

#[test]
fn env_candidate_matches_dotenv_files_only() {
    assert!(is_env_candidate(".env"));
    assert!(is_env_candidate("project/.env.local"));
    assert!(!is_env_candidate("project/env.txt"));
    assert!(!is_env_candidate("project/not.env"));
}

/// The scan's emission bound: a 100k-file install must not be truncated
/// silently, and anything past the bound is counted, not ignored.
#[test]
fn cap_batch_drops_the_overflow_and_reports_it() {
    let mut batch = (0..MAX_QUEUE_SIZE + 3)
        .map(|i| QueuedEvent {
            path: format!("file_{i}.txt"),
            action: FileAction::Modified,
            kind: FileKind::File,
            size: Some(1),
        })
        .collect::<Vec<_>>();

    assert_eq!(cap_batch(&mut batch), 3);
    assert_eq!(batch.len(), MAX_QUEUE_SIZE);
}

#[test]
fn reconciliation_between_scans_carries_kind_and_size_from_the_walk() {
    let root = tempfile::tempdir().unwrap();
    let before = workspace_snapshot(root.path(), root.path());
    std::fs::write(root.path().join("late.txt"), "late write").unwrap();
    let after = workspace_snapshot(root.path(), root.path());

    let events = reconciliation_events(&before, &after);
    assert_eq!(
        events,
        vec![QueuedEvent {
            path: "late.txt".to_string(),
            action: FileAction::Created,
            kind: FileKind::File,
            size: Some(10),
        }]
    );
}

/// A symlink is recorded as a symlink and never dereferenced: its `size` is
/// not the target's, and the walk never descends through it.
#[test]
fn a_symlink_is_recorded_as_a_symlink_with_no_size() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("big.bin");
    std::fs::write(&target, vec![0u8; 1024 * 1024]).unwrap();
    let before = workspace_snapshot(root.path(), root.path());
    std::os::unix::fs::symlink(&target, root.path().join("link")).unwrap();
    let after = workspace_snapshot(root.path(), root.path());

    let events = reconciliation_events(&before, &after);
    assert_eq!(
        events,
        vec![QueuedEvent {
            path: "link".to_string(),
            action: FileAction::Created,
            kind: FileKind::Symlink,
            size: None,
        }]
    );
}

#[test]
fn shutdown_reconciles_unpolled_file_and_security_rows() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let db_path = dir.path().join("session.db");
    let db = Arc::new(DbWriter::open(&db_path, 64).unwrap());
    let profile = SecurityRuleProfile::parse_toml(
        r#"
[profiles.rules.file_create_late]
name = "file_create_late"
action = "allow"
detection_level = "informational"
match = 'file.create.path == "late.txt"'
"#,
    )
    .unwrap();
    let rules = SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User).unwrap();
    let monitor = FsMonitor::start(
        workspace.clone(),
        workspace.clone(),
        Arc::clone(&db),
        Arc::new(std::sync::RwLock::new(Arc::new(rules))),
        empty_trace_state(),
    )
    .unwrap();

    // Do not wait for the next scan. Shutdown itself must be the
    // visibility boundary for this already-materialized file.
    std::fs::write(workspace.join("late.txt"), "late write").unwrap();
    monitor.shutdown_and_join();
    db.shutdown_blocking();

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let joined: (i64, i64) = conn
        .query_row(
            "SELECT COUNT(DISTINCT fs_events.event_id), COUNT(security_rule_events.id)
             FROM fs_events
             JOIN security_rule_events ON security_rule_events.event_id = fs_events.event_id
             WHERE fs_events.path = 'late.txt'
               AND security_rule_events.rule_id = 'profiles.rules.file_create_late'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(joined, (1, 1));
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
        &EmitContext {
            db: &db,
            security_rules: &empty_security_rules(),
            trace_state: &empty_trace_state(),
            strip_prefix: dir.path(),
        },
        &env_event(".env", FileKind::File),
    )
    .await;
    db.shutdown_blocking();

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let file_ref: String = conn
        .query_row("SELECT credential_ref FROM fs_events WHERE path = '.env'", [], |row| {
            row.get(0)
        })
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
        &EmitContext {
            db: &db,
            security_rules: &security_rules,
            trace_state: &empty_trace_state(),
            strip_prefix: dir.path(),
        },
        &QueuedEvent {
            path: "skill.md".to_string(),
            action: FileAction::Created,
            kind: FileKind::File,
            size: Some(7),
        },
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
    trace_state
        .lock()
        .unwrap()
        .register_tool_file_hints("trace-model", [r#"{"cmd":"printf x > /root/openai-two.txt"}"#]);
    FsMonitor::emit(
        &EmitContext {
            db: &db,
            security_rules: &security_rules,
            trace_state: &trace_state,
            strip_prefix: dir.path(),
        },
        &QueuedEvent {
            path: "openai-two.txt".to_string(),
            action: FileAction::Created,
            kind: FileKind::File,
            size: Some(6),
        },
    )
    .await;
    db.shutdown_blocking();

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let (trace_id, credential_ref): (String, Option<String>) = conn
        .query_row(
            "SELECT trace_id, credential_ref FROM fs_events WHERE path = 'openai-two.txt'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let (rule_trace_id, event_credential_ref): (String, Option<String>) = conn
        .query_row(
            "SELECT trace_id, json_extract(event_json, '$.credential_ref') FROM security_rule_events
             WHERE event_id = (SELECT event_id FROM fs_events WHERE path = 'openai-two.txt')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(trace_id, "trace-model");
    assert_eq!(rule_trace_id, "trace-model");
    assert_eq!(credential_ref, None);
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
        &EmitContext {
            db: &db,
            security_rules: &security_rules,
            trace_state: &empty_trace_state(),
            strip_prefix: dir.path(),
        },
        &QueuedEvent {
            path: "blocked.txt".to_string(),
            action: FileAction::Modified,
            kind: FileKind::File,
            size: Some(20),
        },
    )
    .await;
    db.shutdown_blocking();

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let fs_action: String = conn
        .query_row("SELECT action FROM fs_events WHERE path = 'blocked.txt'", [], |row| {
            row.get(0)
        })
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

/// The `.env` broker is the only place the monitor reads guest-controlled
/// bytes. A guest that plants `.env` as a link to a host secret must get
/// nothing: not a brokered reference, not a substitution row, and above all
/// not the host file's contents anywhere near the ledger.
#[tokio::test]
async fn env_symlink_to_a_host_secret_is_never_read_or_brokered() {
    let _lock = crate::credential_broker::TEST_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let capsem_home = dir.path().join("capsem-home");
    let test_store = dir.path().join("credential-store.json");
    let _guard = EnvGuard::install(&capsem_home, dir.path(), &test_store);

    let host_secret = dir.path().join("host-credentials");
    std::fs::write(&host_secret, "AWS_SECRET_ACCESS_KEY=sk-host-only-secret\n").unwrap();
    std::os::unix::fs::symlink(&host_secret, dir.path().join(".env")).unwrap();

    let db = DbWriter::open(&db_path, 64).unwrap();
    FsMonitor::emit(
        &EmitContext {
            db: &db,
            security_rules: &empty_security_rules(),
            trace_state: &empty_trace_state(),
            strip_prefix: dir.path(),
        },
        // The scan's own lstat says symlink; the broker must refuse on that
        // alone, and the O_NOFOLLOW open must refuse it again.
        &env_event(".env", FileKind::Symlink),
    )
    .await;
    db.shutdown_blocking();

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let credential_ref: Option<String> = conn
        .query_row("SELECT credential_ref FROM fs_events WHERE path = '.env'", [], |row| {
            row.get(0)
        })
        .expect("the symlink itself is still a recorded event");
    assert_eq!(credential_ref, None, "a link to a host secret must broker nothing");
    let substitutions: i64 = conn
        .query_row("SELECT COUNT(*) FROM substitution_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(substitutions, 0);
    let db_bytes = std::fs::read(&db_path).unwrap();
    assert!(!String::from_utf8_lossy(&db_bytes).contains("sk-host-only-secret"));
}

/// Even if the kind were wrong, the open refuses to follow the link: the
/// scan's answer and the open are two independent refusals of the same trick.
#[tokio::test]
async fn env_symlink_is_refused_by_the_open_even_if_it_claims_to_be_a_file() {
    let _lock = crate::credential_broker::TEST_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let capsem_home = dir.path().join("capsem-home");
    let test_store = dir.path().join("credential-store.json");
    let _guard = EnvGuard::install(&capsem_home, dir.path(), &test_store);

    let host_secret = dir.path().join("host-credentials");
    std::fs::write(&host_secret, "AWS_SECRET_ACCESS_KEY=sk-host-only-secret\n").unwrap();
    std::os::unix::fs::symlink(&host_secret, dir.path().join(".env")).unwrap();

    let db = DbWriter::open(&db_path, 64).unwrap();
    FsMonitor::emit(
        &EmitContext {
            db: &db,
            security_rules: &empty_security_rules(),
            trace_state: &empty_trace_state(),
            strip_prefix: dir.path(),
        },
        &env_event(".env", FileKind::File),
    )
    .await;
    db.shutdown_blocking();

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let credential_ref: Option<String> = conn
        .query_row("SELECT credential_ref FROM fs_events WHERE path = '.env'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(credential_ref, None);
    let db_bytes = std::fs::read(&db_path).unwrap();
    assert!(!String::from_utf8_lossy(&db_bytes).contains("sk-host-only-secret"));
}

/// The monitor owns its poll loop, so a write lands in the ledger on the next
/// cycle -- shutdown is not the only visibility barrier.
#[tokio::test]
async fn a_hook_write_is_recorded_after_one_poll_cycle_without_shutdown() {
    let dir = tempfile::tempdir().unwrap();
    let (workspace, db_path, db, monitor) = monitor_over_new_workspace(dir.path());

    let hooks = workspace.join(".git/hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    std::fs::write(hooks.join("pre-commit"), "#!/bin/sh\nexfiltrate\n").unwrap();

    // An empty workspace scans in well under a millisecond, so the interval is
    // the 500ms floor. Wait at most three of those, checking as we go, and
    // never shut the monitor down -- shutdown reconciles unconditionally and
    // would hide a loop that never ticks.
    let deadline = std::time::Instant::now() + Duration::from_millis(1500);
    let recorded = loop {
        db.flush().await;
        if let Some(recorded) = recorded_event(&db_path, ".git/hooks/pre-commit") {
            break Some(recorded);
        }
        if std::time::Instant::now() >= deadline {
            break None;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    // Both of these park the calling thread, so they cannot run on the
    // runtime driving this test.
    let teardown_db = Arc::clone(&db);
    tokio::task::spawn_blocking(move || {
        monitor.shutdown_and_join();
        teardown_db.shutdown_blocking();
    })
    .await
    .unwrap();

    let recorded = recorded.expect("a poll cycle must record the hook write without a shutdown");
    assert_eq!((recorded.0.as_str(), recorded.1.as_str()), ("created", "file"));
}
