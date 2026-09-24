use super::*;
use rusqlite::Connection;

mod rollup;

// -- ID generation --

#[test]
fn generate_session_id_format() {
    let id = generate_session_id();
    assert_eq!(id.len(), 20, "id={id}");
    assert!(is_valid_session_id(&id), "id={id}");
}

#[test]
fn two_rapid_calls_differ() {
    let id1 = generate_session_id();
    // Bump PID-based entropy by sleeping briefly.
    std::thread::sleep(std::time::Duration::from_millis(1));
    let id2 = generate_session_id();
    assert_ne!(id1, id2, "ids should differ: {id1} vs {id2}");
}

#[test]
fn is_valid_session_id_accepts_valid() {
    assert!(is_valid_session_id("20260225-143052-a7f3"));
    assert!(is_valid_session_id("20260101-000000-0000"));
    assert!(is_valid_session_id("20260225-235959-ffff"));
}

#[test]
fn is_valid_session_id_rejects_invalid() {
    assert!(!is_valid_session_id("default"));
    assert!(!is_valid_session_id("cli"));
    assert!(!is_valid_session_id(""));
    assert!(!is_valid_session_id("2026022514305-a7f3")); // missing digit
    assert!(!is_valid_session_id("20260225-14305-a7f3x")); // wrong length
    assert!(!is_valid_session_id("XXXXXXXX-XXXXXX-XXXX")); // not digits
}

// -- SessionIndex CRUD --

fn sample_record(id: &str, status: &str) -> SessionRecord {
    SessionRecord {
        id: id.to_string(),
        mode: "gui".to_string(),
        command: None,
        status: status.to_string(),
        created_at: "2026-02-25T14:30:52Z".to_string(),
        stopped_at: None,
        scratch_disk_size_gb: 16,
        ram_bytes: 4 * 1024 * 1024 * 1024,
        total_requests: 0,
        allowed_requests: 0,
        denied_requests: 0,
        total_input_tokens: 0,
        total_output_tokens: 0,
        total_estimated_cost: 0.0,
        total_tool_calls: 0,
        total_file_events: 0,
        storage_mode: "block".to_string(),
        rootfs_hash: None,
        rootfs_version: None,
        forked_from: None,
        persistent: false,
        exec_count: 0,
        audit_event_count: 0,
    }
}

#[test]
fn open_creates_schema() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.db");
    let idx = SessionIndex::open(&path).unwrap();
    assert_eq!(idx.count().unwrap(), 0);
    assert!(path.exists());
}

#[test]
fn open_preserves_data() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.db");
    {
        let idx = SessionIndex::open(&path).unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "running"))
            .unwrap();
    }
    let idx = SessionIndex::open(&path).unwrap();
    assert_eq!(idx.count().unwrap(), 1);
}

#[test]
fn open_in_memory_works() {
    let idx = SessionIndex::open_in_memory().unwrap();
    assert_eq!(idx.count().unwrap(), 0);
}

#[test]
fn create_and_recent() {
    let idx = SessionIndex::open_in_memory().unwrap();
    idx.create_session(&sample_record("20260225-143052-a7f3", "running"))
        .unwrap();
    let records = idx.recent(1).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].id, "20260225-143052-a7f3");
    assert_eq!(records[0].mode, "gui");
    assert_eq!(records[0].status, "running");
}

#[test]
fn create_duplicate_returns_error() {
    let idx = SessionIndex::open_in_memory().unwrap();
    idx.create_session(&sample_record("20260225-143052-a7f3", "running"))
        .unwrap();
    let result = idx.create_session(&sample_record("20260225-143052-a7f3", "running"));
    assert!(result.is_err());
}

#[test]
fn recent_newest_first() {
    let idx = SessionIndex::open_in_memory().unwrap();
    for (i, ts) in ["2026-02-25T10:00:00Z", "2026-02-25T12:00:00Z", "2026-02-25T11:00:00Z"]
        .iter()
        .enumerate()
    {
        let mut rec = sample_record(&format!("20260225-{i:06}-0000"), "stopped");
        rec.created_at = ts.to_string();
        idx.create_session(&rec).unwrap();
    }
    let records = idx.recent(10).unwrap();
    assert_eq!(records[0].created_at, "2026-02-25T12:00:00Z");
    assert_eq!(records[1].created_at, "2026-02-25T11:00:00Z");
    assert_eq!(records[2].created_at, "2026-02-25T10:00:00Z");
}

#[test]
fn recent_respects_limit() {
    let idx = SessionIndex::open_in_memory().unwrap();
    for i in 0..5 {
        let mut rec = sample_record(&format!("20260225-{i:06}-0000"), "stopped");
        rec.created_at = format!("2026-02-25T{i:02}:00:00Z");
        idx.create_session(&rec).unwrap();
    }
    assert_eq!(idx.recent(2).unwrap().len(), 2);
}

#[test]
fn recent_empty_db() {
    let idx = SessionIndex::open_in_memory().unwrap();
    assert!(idx.recent(10).unwrap().is_empty());
}

#[test]
fn update_status_works() {
    let idx = SessionIndex::open_in_memory().unwrap();
    idx.create_session(&sample_record("20260225-143052-a7f3", "running"))
        .unwrap();
    idx.update_status("20260225-143052-a7f3", "stopped", Some("2026-02-25T15:00:00Z"))
        .unwrap();
    let records = idx.recent(1).unwrap();
    assert_eq!(records[0].status, "stopped");
    assert_eq!(records[0].stopped_at.as_deref(), Some("2026-02-25T15:00:00Z"));
}

#[test]
fn update_status_nonexistent_is_noop() {
    let idx = SessionIndex::open_in_memory().unwrap();
    // Should not crash.
    idx.update_status("nonexistent", "stopped", None).unwrap();
}

#[test]
fn count_correct() {
    let idx = SessionIndex::open_in_memory().unwrap();
    assert_eq!(idx.count().unwrap(), 0);
    idx.create_session(&sample_record("20260225-143052-a7f3", "running"))
        .unwrap();
    assert_eq!(idx.count().unwrap(), 1);
    idx.create_session(&sample_record("20260225-143053-b8e4", "stopped"))
        .unwrap();
    assert_eq!(idx.count().unwrap(), 2);
}

// -- Crash recovery --

#[test]
fn mark_running_as_crashed() {
    let idx = SessionIndex::open_in_memory().unwrap();
    idx.create_session(&sample_record("20260225-143052-a7f3", "running"))
        .unwrap();
    idx.create_session(&sample_record("20260225-143053-b8e4", "running"))
        .unwrap();
    idx.create_session(&sample_record("20260225-143054-c9d5", "stopped"))
        .unwrap();

    let count = idx.mark_running_as_crashed().unwrap();
    assert_eq!(count, 2);

    let records = idx.recent(10).unwrap();
    for r in &records {
        if r.id == "20260225-143054-c9d5" {
            assert_eq!(r.status, "stopped");
        } else {
            assert_eq!(r.status, "crashed");
        }
    }
}

#[test]
fn mark_running_as_crashed_ignores_stopped() {
    let idx = SessionIndex::open_in_memory().unwrap();
    idx.create_session(&sample_record("20260225-143052-a7f3", "stopped"))
        .unwrap();
    idx.create_session(&sample_record("20260225-143053-b8e4", "crashed"))
        .unwrap();
    let count = idx.mark_running_as_crashed().unwrap();
    assert_eq!(count, 0);
}

#[test]
fn mark_running_as_crashed_empty_db() {
    let idx = SessionIndex::open_in_memory().unwrap();
    let count = idx.mark_running_as_crashed().unwrap();
    assert_eq!(count, 0);
}

// -- Disk culling helper --

#[test]
fn stopped_sessions_oldest_first() {
    let idx = SessionIndex::open_in_memory().unwrap();

    let mut s1 = sample_record("20260225-100000-0000", "stopped");
    s1.created_at = "2026-02-25T10:00:00Z".to_string();
    idx.create_session(&s1).unwrap();

    let mut s2 = sample_record("20260225-120000-0000", "crashed");
    s2.created_at = "2026-02-25T12:00:00Z".to_string();
    idx.create_session(&s2).unwrap();

    let mut s3 = sample_record("20260225-080000-0000", "running");
    s3.created_at = "2026-02-25T08:00:00Z".to_string();
    idx.create_session(&s3).unwrap();

    let stopped = idx.stopped_sessions_oldest_first().unwrap();
    assert_eq!(stopped.len(), 2); // running excluded
    assert_eq!(stopped[0].id, "20260225-100000-0000");
    assert_eq!(stopped[1].id, "20260225-120000-0000");
}

// -- epoch_to_iso --

#[test]
fn epoch_to_iso_unix_epoch() {
    assert_eq!(epoch_to_iso(0), "1970-01-01T00:00:00Z");
}

#[test]
fn epoch_to_iso_known_date() {
    // 2026-02-25T14:30:52Z = known epoch
    let iso = epoch_to_iso(1772126052);
    assert!(iso.starts_with("2026-"), "iso={iso}");
}

// -- Schema version --

#[test]
fn schema_version_is_set() {
    let idx = SessionIndex::open_in_memory().unwrap();
    let version: u32 = idx
        .conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);
}

#[test]
fn schema_upgrade_from_v0() {
    // Simulate a v0 DB (no user_version set = 0).
    let conn = Connection::open_in_memory().unwrap();
    // Create old-style sessions table without new columns.
    conn.execute_batch("CREATE TABLE sessions (id TEXT PRIMARY KEY, mode TEXT NOT NULL)")
        .unwrap();
    conn.execute("INSERT INTO sessions (id, mode) VALUES ('old', 'gui')", [])
        .unwrap();
    // Now ensure_schema should drop and recreate (v0 < v2 path).
    SessionIndex::ensure_schema(&conn).unwrap();
    let version: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
    assert_eq!(version, SCHEMA_VERSION);
    // Old data is gone (clean slate).
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn schema_upgrade_from_v1() {
    // v1 < v2, so same drop+recreate behavior.
    let conn = Connection::open_in_memory().unwrap();
    conn.pragma_update(None, "user_version", 1u32).unwrap();
    conn.execute_batch("CREATE TABLE sessions (id TEXT PRIMARY KEY)")
        .unwrap();
    SessionIndex::ensure_schema(&conn).unwrap();
    let version: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
    assert_eq!(version, SCHEMA_VERSION);
}

#[test]
fn schema_same_version_preserves_data() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.db");
    {
        let idx = SessionIndex::open(&path).unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "running"))
            .unwrap();
    }
    // Reopen -- same version, data preserved.
    let idx = SessionIndex::open(&path).unwrap();
    assert_eq!(idx.count().unwrap(), 1);
}

// -- New columns default to zero --

#[test]
fn new_columns_default_to_zero() {
    let idx = SessionIndex::open_in_memory().unwrap();
    idx.create_session(&sample_record("20260225-143052-a7f3", "running"))
        .unwrap();
    let records = idx.recent(1).unwrap();
    assert_eq!(records[0].total_input_tokens, 0);
    assert_eq!(records[0].total_output_tokens, 0);
    assert_eq!(records[0].total_estimated_cost, 0.0);
    assert_eq!(records[0].total_tool_calls, 0);
    assert_eq!(records[0].total_file_events, 0);
}

// -- replace_*_usage: per-session usage rows in main.db --

/// `(key, call_count)` rows of one usage table, in key order.
fn usage_rows(idx: &SessionIndex, sql: &str) -> Vec<(String, i64)> {
    let mut stmt = idx.conn.prepare(sql).unwrap();
    let rows = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    rows
}

#[test]
fn replace_ai_usage_writes_one_row_per_provider() {
    let idx = SessionIndex::open_in_memory().unwrap();
    idx.create_session(&sample_record("20260225-143052-a7f3", "stopped"))
        .unwrap();

    let usage = vec![
        ProviderSummary {
            provider: "anthropic".into(),
            call_count: 10,
            input_tokens: 5000,
            output_tokens: 2000,
            estimated_cost: 0.10,
            total_duration_ms: 3000,
        },
        ProviderSummary {
            provider: "google".into(),
            call_count: 5,
            input_tokens: 2000,
            output_tokens: 1000,
            estimated_cost: 0.05,
            total_duration_ms: 1500,
        },
    ];
    idx.replace_ai_usage("20260225-143052-a7f3", &usage).unwrap();

    let (tokens, cost): (i64, f64) = idx
        .conn
        .query_row(
            "SELECT input_tokens, estimated_cost FROM ai_usage
             WHERE session_id = '20260225-143052-a7f3' AND provider = 'anthropic'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(tokens, 5000);
    assert!((cost - 0.10).abs() < 1e-9);
    assert_eq!(
        usage_rows(&idx, "SELECT provider, call_count FROM ai_usage ORDER BY provider"),
        vec![("anthropic".to_string(), 10), ("google".to_string(), 5)]
    );
}

#[test]
fn replace_ai_usage_replaces_old_data() {
    let idx = SessionIndex::open_in_memory().unwrap();
    idx.create_session(&sample_record("20260225-143052-a7f3", "stopped"))
        .unwrap();

    let old = vec![ProviderSummary {
        provider: "anthropic".into(),
        call_count: 10,
        input_tokens: 5000,
        output_tokens: 2000,
        estimated_cost: 0.10,
        total_duration_ms: 3000,
    }];
    idx.replace_ai_usage("20260225-143052-a7f3", &old).unwrap();

    let new = vec![ProviderSummary {
        provider: "openai".into(),
        call_count: 20,
        input_tokens: 8000,
        output_tokens: 4000,
        estimated_cost: 0.30,
        total_duration_ms: 5000,
    }];
    idx.replace_ai_usage("20260225-143052-a7f3", &new).unwrap();

    assert_eq!(
        usage_rows(&idx, "SELECT provider, call_count FROM ai_usage ORDER BY provider"),
        vec![("openai".to_string(), 20)]
    );
}

#[test]
fn replace_tool_usage_writes_one_row_per_tool() {
    let idx = SessionIndex::open_in_memory().unwrap();
    idx.create_session(&sample_record("20260225-143052-a7f3", "stopped"))
        .unwrap();

    let usage = vec![
        ToolSummary {
            tool_name: "read_file".into(),
            call_count: 50,
            total_bytes: 100_000,
            total_duration_ms: 2000,
        },
        ToolSummary {
            tool_name: "write_file".into(),
            call_count: 30,
            total_bytes: 50_000,
            total_duration_ms: 1500,
        },
    ];
    idx.replace_tool_usage("20260225-143052-a7f3", &usage).unwrap();

    assert_eq!(
        usage_rows(&idx, "SELECT tool_name, call_count FROM tool_usage ORDER BY tool_name"),
        vec![("read_file".to_string(), 50), ("write_file".to_string(), 30)]
    );
}

#[test]
fn replace_mcp_usage_writes_one_row_per_server_tool() {
    let idx = SessionIndex::open_in_memory().unwrap();
    idx.create_session(&sample_record("20260225-143052-a7f3", "stopped"))
        .unwrap();

    let usage = vec![
        McpToolSummary {
            tool_name: "github__search".into(),
            server_name: "github".into(),
            call_count: 15,
            total_bytes: 30_000,
            total_duration_ms: 4500,
        },
        McpToolSummary {
            tool_name: "fs__read".into(),
            server_name: "filesystem".into(),
            call_count: 8,
            total_bytes: 10_000,
            total_duration_ms: 800,
        },
    ];
    idx.replace_mcp_usage("20260225-143052-a7f3", &usage).unwrap();

    assert_eq!(
        usage_rows(
            &idx,
            "SELECT server_name || '/' || tool_name, call_count FROM mcp_usage ORDER BY server_name"
        ),
        vec![
            ("filesystem/fs__read".to_string(), 8),
            ("github/github__search".to_string(), 15)
        ]
    );
}

// -- Schema migration v2->v3 --

#[test]
fn schema_upgrade_from_v4_preserves_data() {
    let conn = Connection::open_in_memory().unwrap();
    // Create a v4 schema manually.
    conn.pragma_update(None, "user_version", 4u32).unwrap();
    conn.execute_batch(
        "
        CREATE TABLE sessions (
            id TEXT PRIMARY KEY, mode TEXT NOT NULL, command TEXT,
            status TEXT NOT NULL DEFAULT 'running', created_at TEXT NOT NULL,
            stopped_at TEXT, scratch_disk_size_gb INTEGER NOT NULL DEFAULT 16,
            ram_bytes INTEGER NOT NULL DEFAULT 4294967296,
            total_requests INTEGER NOT NULL DEFAULT 0,
            allowed_requests INTEGER NOT NULL DEFAULT 0,
            denied_requests INTEGER NOT NULL DEFAULT 0,
            total_input_tokens INTEGER NOT NULL DEFAULT 0,
            total_output_tokens INTEGER NOT NULL DEFAULT 0,
            total_estimated_cost REAL NOT NULL DEFAULT 0.0,
            total_tool_calls INTEGER NOT NULL DEFAULT 0,
            total_file_events INTEGER NOT NULL DEFAULT 0,
            compressed_size_bytes INTEGER,
            vacuumed_at TEXT,
            storage_mode TEXT NOT NULL DEFAULT 'block',
            rootfs_hash TEXT,
            rootfs_version TEXT
        );
    ",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO sessions (id, mode, status, created_at) VALUES ('test-v4', 'gui', 'stopped', '2026-01-01T00:00:00Z')",
        [],
    ).unwrap();

    // Migrate.
    SessionIndex::ensure_schema(&conn).unwrap();

    // Check version bumped.
    let version: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
    assert_eq!(version, SCHEMA_VERSION);

    // New columns exist with NULL/default defaults.
    let forked_from: Option<String> = conn
        .query_row("SELECT forked_from FROM sessions WHERE id = 'test-v4'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert!(forked_from.is_none());

    let persistent: bool = conn
        .query_row("SELECT persistent FROM sessions WHERE id = 'test-v4'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert!(!persistent);

    // A v4 ledger really had the vacuum columns, so this is the drop path.
    assert!(!has_column(&conn, "compressed_size_bytes"));
    assert!(!has_column(&conn, "vacuumed_at"));
    let audit_event_count: i64 = conn
        .query_row(
            "SELECT audit_event_count FROM sessions WHERE id = 'test-v4'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(audit_event_count, 0);
}

/// A session that reached the dead `vacuumed` state becomes a stopped one.
#[test]
fn schema_upgrade_rewrites_the_vacuumed_state() {
    let conn = Connection::open_in_memory().unwrap();
    conn.pragma_update(None, "user_version", 7u32).unwrap();
    conn.execute_batch(crate::session_index::SESSION_SCHEMA).unwrap();
    conn.execute_batch(
        "ALTER TABLE sessions ADD COLUMN compressed_size_bytes INTEGER;
         ALTER TABLE sessions ADD COLUMN vacuumed_at TEXT;",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO sessions (id, mode, status, created_at, vacuumed_at, compressed_size_bytes)
         VALUES ('test-v7', 'gui', 'vacuumed', '2026-01-01T00:00:00Z', '2026-01-02T00:00:00Z', 4096)",
        [],
    )
    .unwrap();

    SessionIndex::ensure_schema(&conn).unwrap();

    let status: String = conn
        .query_row("SELECT status FROM sessions WHERE id = 'test-v7'", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        status, "stopped",
        "a state no session can reach is not kept in the data"
    );
    assert!(!has_column(&conn, "vacuumed_at"));
    assert!(!has_column(&conn, "compressed_size_bytes"));
}

#[test]
fn schema_upgrade_from_v2_preserves_data() {
    let conn = Connection::open_in_memory().unwrap();
    // Create a v2 schema manually.
    conn.pragma_update(None, "user_version", 2u32).unwrap();
    conn.execute_batch("
        CREATE TABLE sessions (
            id TEXT PRIMARY KEY, mode TEXT NOT NULL, command TEXT,
            status TEXT NOT NULL DEFAULT 'running', created_at TEXT NOT NULL,
            stopped_at TEXT, scratch_disk_size_gb INTEGER NOT NULL DEFAULT 16,
            ram_bytes INTEGER NOT NULL DEFAULT 4294967296,
            total_requests INTEGER NOT NULL DEFAULT 0,
            allowed_requests INTEGER NOT NULL DEFAULT 0,
            denied_requests INTEGER NOT NULL DEFAULT 0,
            total_input_tokens INTEGER NOT NULL DEFAULT 0,
            total_output_tokens INTEGER NOT NULL DEFAULT 0,
            total_estimated_cost REAL NOT NULL DEFAULT 0.0,
            total_tool_calls INTEGER NOT NULL DEFAULT 0,
            total_file_events INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE ai_usage (session_id TEXT, provider TEXT, call_count INTEGER DEFAULT 0, input_tokens INTEGER DEFAULT 0, output_tokens INTEGER DEFAULT 0, estimated_cost REAL DEFAULT 0.0, total_duration_ms INTEGER DEFAULT 0, PRIMARY KEY (session_id, provider));
        CREATE TABLE tool_usage (session_id TEXT, tool_name TEXT, call_count INTEGER DEFAULT 0, total_bytes INTEGER DEFAULT 0, total_duration_ms INTEGER DEFAULT 0, PRIMARY KEY (session_id, tool_name));
        CREATE TABLE mcp_usage (session_id TEXT, tool_name TEXT, server_name TEXT, call_count INTEGER DEFAULT 0, total_bytes INTEGER DEFAULT 0, total_duration_ms INTEGER DEFAULT 0, PRIMARY KEY (session_id, tool_name));
    ").unwrap();
    conn.execute(
        "INSERT INTO sessions (id, mode, status, created_at) VALUES ('test-id', 'gui', 'stopped', '2026-01-01T00:00:00Z')",
        [],
    ).unwrap();

    // Migrate.
    SessionIndex::ensure_schema(&conn).unwrap();

    // Check version bumped.
    let version: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
    assert_eq!(version, SCHEMA_VERSION);

    // Old data preserved.
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);

    // Every column the current shape has, including the two the v6->v7 step
    // adds: the branch-per-version migration used to stamp SCHEMA_VERSION
    // after one jump, leaving older ledgers current-but-incomplete.
    let exec_count: i64 = conn
        .query_row("SELECT exec_count FROM sessions WHERE id = 'test-id'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(exec_count, 0);
    assert!(
        !has_column(&conn, "compressed_size_bytes"),
        "the vacuum columns are gone"
    );
    assert!(!has_column(&conn, "vacuumed_at"), "the vacuum columns are gone");
}

/// Whether `sessions` has a column, for the migration assertions.
fn has_column(conn: &Connection, column: &str) -> bool {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('sessions') WHERE name = ?1)",
        rusqlite::params![column],
        |row| row.get(0),
    )
    .unwrap()
}

// -- New lifecycle methods --

#[test]
fn mark_terminated_sets_status() {
    let idx = SessionIndex::open_in_memory().unwrap();
    idx.create_session(&sample_record("20260225-143052-a7f3", "stopped"))
        .unwrap();
    idx.mark_terminated("20260225-143052-a7f3").unwrap();

    let records = idx.recent(1).unwrap();
    assert_eq!(records[0].status, "terminated");
}

#[test]
fn sessions_by_status_filters_correctly() {
    let idx = SessionIndex::open_in_memory().unwrap();
    idx.create_session(&sample_record("20260225-143052-a7f3", "stopped"))
        .unwrap();
    let mut r2 = sample_record("20260225-143053-b8e4", "running");
    r2.created_at = "2026-02-25T14:30:53Z".to_string();
    idx.create_session(&r2).unwrap();
    let mut r3 = sample_record("20260225-143054-c9d5", "stopped");
    r3.created_at = "2026-02-25T14:30:54Z".to_string();
    idx.create_session(&r3).unwrap();

    let stopped = idx.sessions_by_status("stopped").unwrap();
    assert_eq!(stopped.len(), 2);
    let running = idx.sessions_by_status("running").unwrap();
    assert_eq!(running.len(), 1);
    let terminated = idx.sessions_by_status("terminated").unwrap();
    assert_eq!(terminated.len(), 0);
}

#[test]
fn purge_terminated_older_than_days() {
    let idx = SessionIndex::open_in_memory().unwrap();

    // Old terminated session.
    let mut old = sample_record("20200101-120000-0000", "terminated");
    old.created_at = "2020-01-01T12:00:00Z".to_string();
    idx.create_session(&old).unwrap();

    // Recent terminated session (use a date far in the future to avoid flaking).
    let mut recent = sample_record("20260225-143052-a7f3", "terminated");
    recent.created_at = "2099-01-01T00:00:00Z".to_string();
    idx.create_session(&recent).unwrap();

    // Non-terminated session.
    let mut stopped = sample_record("20200101-130000-0000", "stopped");
    stopped.created_at = "2020-01-01T13:00:00Z".to_string();
    idx.create_session(&stopped).unwrap();

    let purged = idx.purge_terminated_older_than_days(7).unwrap();
    assert_eq!(purged, 1); // only old terminated
    assert_eq!(idx.count().unwrap(), 2); // recent terminated + stopped remain
}

#[test]
fn full_lifecycle_running_to_terminated() {
    let idx = SessionIndex::open_in_memory().unwrap();
    idx.create_session(&sample_record("20260225-143052-a7f3", "running"))
        .unwrap();

    // running -> stopped
    idx.update_status("20260225-143052-a7f3", "stopped", Some("2026-02-25T15:00:00Z"))
        .unwrap();
    assert_eq!(idx.recent(1).unwrap()[0].status, "stopped");

    // stopped -> terminated
    idx.mark_terminated("20260225-143052-a7f3").unwrap();
    assert_eq!(idx.recent(1).unwrap()[0].status, "terminated");

    // Row still exists in the audit trail.
    assert_eq!(idx.count().unwrap(), 1);
}

#[test]
fn checkpoint_succeeds_on_in_memory_db() {
    let idx = SessionIndex::open_in_memory().unwrap();
    // Should not error (even though in-memory WAL is a no-op).
    idx.checkpoint().unwrap();
}

#[test]
fn stopped_sessions_includes_crashed() {
    let idx = SessionIndex::open_in_memory().unwrap();

    let mut s1 = sample_record("20260225-100000-0000", "stopped");
    s1.created_at = "2026-02-25T10:00:00Z".to_string();
    idx.create_session(&s1).unwrap();

    let mut s2 = sample_record("20260225-110000-0000", "crashed");
    s2.created_at = "2026-02-25T11:00:00Z".to_string();
    idx.create_session(&s2).unwrap();

    let mut s3 = sample_record("20260225-120000-0000", "terminated");
    s3.created_at = "2026-02-25T12:00:00Z".to_string();
    idx.create_session(&s3).unwrap();

    let stopped = idx.stopped_sessions_oldest_first().unwrap();
    assert_eq!(stopped.len(), 2); // stopped + crashed, not terminated
    assert_eq!(stopped[0].id, "20260225-100000-0000");
    assert_eq!(stopped[1].id, "20260225-110000-0000");
}

// -- query_raw --

#[test]
fn query_raw_returns_columnar_json() {
    let idx = SessionIndex::open_in_memory().unwrap();
    idx.create_session(&sample_record("20260225-143052-a7f3", "running"))
        .unwrap();

    let json_str = idx.query_raw("SELECT id, mode, status FROM sessions", &[]).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed["columns"], serde_json::json!(["id", "mode", "status"]));
    assert_eq!(parsed["rows"].as_array().unwrap().len(), 1);
    assert_eq!(parsed["rows"][0][0], "20260225-143052-a7f3");
}

#[test]
fn query_raw_with_bind_params() {
    let idx = SessionIndex::open_in_memory().unwrap();
    idx.create_session(&sample_record("20260225-143052-a7f3", "running"))
        .unwrap();
    let mut r2 = sample_record("20260225-143053-b8e4", "stopped");
    r2.created_at = "2026-02-25T14:30:53Z".to_string();
    idx.create_session(&r2).unwrap();

    let params = vec![serde_json::json!("stopped")];
    let json_str = idx
        .query_raw("SELECT id FROM sessions WHERE status = ?", &params)
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed["rows"].as_array().unwrap().len(), 1);
    assert_eq!(parsed["rows"][0][0], "20260225-143053-b8e4");
}

#[test]
fn query_raw_empty_result() {
    let idx = SessionIndex::open_in_memory().unwrap();
    let json_str = idx.query_raw("SELECT id FROM sessions", &[]).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed["rows"].as_array().unwrap().len(), 0);
    assert_eq!(parsed["columns"], serde_json::json!(["id"]));
}

#[test]
fn query_raw_with_limit_param() {
    let idx = SessionIndex::open_in_memory().unwrap();
    for i in 0..5 {
        let mut rec = sample_record(&format!("20260225-{i:06}-0000"), "running");
        rec.created_at = format!("2026-02-25T{i:02}:00:00Z");
        idx.create_session(&rec).unwrap();
    }

    let params = vec![serde_json::json!(2)];
    let json_str = idx.query_raw("SELECT id FROM sessions LIMIT ?", &params).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed["rows"].as_array().unwrap().len(), 2);
}

// -- query_raw read-only enforcement (PRAGMA query_only) --

#[test]
fn query_raw_rejects_insert() {
    let idx = SessionIndex::open_in_memory().unwrap();
    idx.create_session(&sample_record("20260225-143052-a7f3", "running"))
        .unwrap();

    let result = idx.query_raw(
        "INSERT INTO sessions (id, mode, status, created_at) VALUES ('evil', 'gui', 'running', '2026-01-01T00:00:00Z')",
        &[],
    );
    assert!(result.is_err(), "INSERT must be rejected by PRAGMA query_only");
}

#[test]
fn query_raw_rejects_semicolon_injection() {
    let idx = SessionIndex::open_in_memory().unwrap();
    idx.create_session(&sample_record("20260225-143052-a7f3", "running"))
        .unwrap();

    // Multi-statement: first is SELECT (passes validate_select_only),
    // second is DROP TABLE (must be caught by PRAGMA query_only).
    let _result = idx.query_raw("SELECT 1; DROP TABLE sessions", &[]);
    // Either the prepare or execute step should reject this.
    // The SELECT may succeed but DROP must not execute.
    // Verify sessions table is intact regardless.
    let count = idx.count().unwrap();
    assert_eq!(count, 1, "sessions table must not be dropped");
}

#[test]
fn query_raw_select_works() {
    let idx = SessionIndex::open_in_memory().unwrap();
    idx.create_session(&sample_record("20260225-143052-a7f3", "running"))
        .unwrap();

    let result = idx.query_raw("SELECT COUNT(*) FROM sessions", &[]);
    assert!(result.is_ok(), "SELECT must succeed: {:?}", result);
    let parsed: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
    assert_eq!(parsed["rows"][0][0], 1);
}

#[test]
fn query_raw_other_methods_still_write() {
    let idx = SessionIndex::open_in_memory().unwrap();
    idx.create_session(&sample_record("20260225-143052-a7f3", "running"))
        .unwrap();

    // Call query_raw (sets PRAGMA query_only ON then OFF).
    let _ = idx.query_raw("SELECT 1", &[]);

    // Internal write methods must still work after query_raw restored
    // the connection to read-write mode.
    idx.update_status("20260225-143052-a7f3", "stopped", Some("2026-02-25T15:00:00Z"))
        .unwrap();
    let records = idx.recent(1).unwrap();
    assert_eq!(records[0].status, "stopped");
}

#[test]
fn query_raw_restores_write_on_error() {
    let idx = SessionIndex::open_in_memory().unwrap();
    idx.create_session(&sample_record("20260225-143052-a7f3", "running"))
        .unwrap();

    // Trigger an error inside query_raw (bad SQL).
    let _ = idx.query_raw("INSERT INTO sessions VALUES ('x')", &[]);

    // PRAGMA query_only must be restored to OFF despite the error.
    idx.create_session(&sample_record("20260225-143053-b8e4", "running"))
        .unwrap();
    assert_eq!(idx.count().unwrap(), 2);
}

// -- Fork metadata and MCP rollups --
//
// These lived in an inline `mod retention_tests` at the bottom of
// session_index.rs, beside the terminators this change deleted. They belong
// in the sibling tests.rs the rest of the crate uses.

/// A session with the content counters a rollup assertion needs.
fn content_session(
    id: &str,
    created_at: &str,
    status: &str,
    tokens: u64,
    tool_calls: u64,
    requests: u64,
) -> SessionRecord {
    let mut record = sample_record(id, status);
    record.created_at = created_at.to_string();
    record.total_input_tokens = tokens;
    record.total_tool_calls = tool_calls;
    record.total_requests = requests;
    record
}

#[test]
fn session_with_forked_from_roundtrips() {
    let idx = SessionIndex::open_in_memory().unwrap();
    let mut s = content_session("20260326-100000-0001", "2026-03-26T10:00:00Z", "running", 0, 0, 0);
    s.forked_from = Some("my-image".into());
    s.persistent = true;
    idx.create_session(&s).unwrap();

    let sessions = idx.recent(10).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].forked_from.as_deref(), Some("my-image"));
    assert!(sessions[0].persistent);
}

#[test]
fn v6_schema_has_forked_from_and_persistent() {
    // Verify the schema includes the new columns by inserting and querying
    let idx = SessionIndex::open_in_memory().unwrap();
    let mut s = content_session("20260326-100000-0001", "2026-03-26T10:00:00Z", "running", 0, 0, 0);
    s.forked_from = Some("test-img".into());
    s.persistent = true;
    idx.create_session(&s).unwrap();

    // Raw query to verify columns exist
    let (src_img, pers): (Option<String>, bool) = idx
        .conn
        .query_row(
            "SELECT forked_from, persistent FROM sessions WHERE id = ?1",
            params!["20260326-100000-0001"],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(src_img.as_deref(), Some("test-img"));
    assert!(pers);
}

#[test]
fn mcp_usage_keys_the_same_tool_name_by_server() {
    let idx = SessionIndex::open_in_memory().unwrap();
    let s1 = content_session("s1", "2026-03-01T10:00:00Z", "stopped", 10, 5, 1);
    idx.create_session(&s1).unwrap();

    // Same tool_name "search" from two servers in one session: the usage key
    // includes the server, so neither row overwrites the other.
    let search = |server_name: &str, call_count| McpToolSummary {
        tool_name: "search".into(),
        server_name: server_name.into(),
        call_count,
        total_bytes: 100,
        total_duration_ms: 50,
    };
    idx.replace_mcp_usage("s1", &[search("github", 3), search("jira", 2)])
        .unwrap();

    assert_eq!(
        usage_rows(
            &idx,
            "SELECT server_name, call_count FROM mcp_usage WHERE tool_name = 'search' ORDER BY server_name"
        ),
        vec![("github".to_string(), 3), ("jira".to_string(), 2)],
        "same tool_name from different servers should be separate rows"
    );
}
