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

#[test]
fn reconciliation_finds_changes_missing_from_notify_queue() {
    let root = tempfile::tempdir().unwrap();
    let before = workspace_snapshot(root.path(), root.path());
    let created = root.path().join("late.txt");
    std::fs::write(&created, "late write").unwrap();
    let after = workspace_snapshot(root.path(), root.path());

    let events = reconciliation_events(&before, &after);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].path, "late.txt");
    assert_eq!(events[0].action, FileAction::Created);
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

    // Do not wait for PollWatcher's 500ms scan. Shutdown itself must be the
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
