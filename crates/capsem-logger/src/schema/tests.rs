use super::*;
use capsem_telemetry::db::{
    DB_SQLITE_FILE_SIZE_BYTES, DB_SQLITE_MMAP_BUDGET_CHECKS_TOTAL, DB_SQLITE_MMAP_CONFIG_BYTES,
    DB_SQLITE_MMAP_COVERAGE_RATIO, DB_SQLITE_MMAP_EFFECTIVE_BYTES, DB_SQLITE_WAL_SIZE_BYTES,
};
use rusqlite::OpenFlags;

mod archive;

fn columns_for_schema(conn: &Connection, schema: &str, table: &str) -> BTreeSet<String> {
    let pragma = if schema == "main" {
        format!("PRAGMA table_info({table})")
    } else {
        format!("PRAGMA {schema}.table_info({table})")
    };
    let mut stmt = conn.prepare(&pragma).unwrap();
    stmt.query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .collect::<Result<BTreeSet<_>, _>>()
        .unwrap()
}

/// Rows the writer of the ledger at `path` still holds in its memory schema.
///
/// Opens its own connection to the shared-cache memory database, so a test
/// can watch the writer's `mem` without a hook into the writer thread.
/// `read_uncommitted` keeps it off the writer's table locks.
pub(crate) fn memory_row_count_for_tests(path: &std::path::Path, table: &str) -> i64 {
    use rusqlite::OpenFlags;
    let conn = Connection::open_with_flags(
        ":memory:",
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE | OpenFlags::SQLITE_OPEN_URI,
    )
    .expect("open probe connection");
    let uri = memory_uri_for_path(path).replace('\'', "''");
    conn.execute_batch(&format!(
        "ATTACH DATABASE '{uri}' AS probe; PRAGMA read_uncommitted = ON;"
    ))
    .expect("attach the writer's memory schema");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match conn.query_row(&format!("SELECT COUNT(*) FROM probe.{table}"), [], |row| row.get(0)) {
            Ok(count) => return count,
            Err(_) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            Err(error) => panic!("count probe.{table}: {error}"),
        }
    }
}

#[test]
fn create_tables_succeeds() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
}

#[test]
fn repeated_event_counters_require_exact_positive_integers() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    assert!(conn
        .execute(
            "INSERT INTO security_rule_runs
             (event_type, rule_id, rule_action, detection_level, rule_json,
              count, first_timestamp_unix_ms, last_timestamp_unix_ms)
             VALUES ('dns.query', 'rule', 'allow', 'none', '{}', 1.5, 1, 2)",
            [],
        )
        .is_err());
    assert!(conn
        .execute(
            "INSERT INTO security_decision_runs
             (event_type, stage, actor, previous_decision, requested_decision,
              effective_decision, count, first_timestamp_unix_ms, last_timestamp_unix_ms)
             VALUES ('dns.query', 'rule', 'rule', 'allow', 'allow', 'allow', 1.5, 1, 2)",
            [],
        )
        .is_err());
}

#[test]
fn create_tables_idempotent() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    create_tables(&conn).unwrap();
}

#[test]
fn db_mem_tables_match_schema() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    create_memory_tables(&conn, &memory_uri_for_name("db_mem_tables_match_schema")).unwrap();

    for (table, _) in READY_SCHEMA_COLUMNS {
        let main_columns = columns_for_schema(&conn, "main", table);
        if is_disk_only_table(table) {
            let mem_columns = columns_for_schema(&conn, MEMORY_SCHEMA, table);
            assert!(
                mem_columns.is_empty(),
                "{table} must stay disk-only; blob payloads are bounded durable storage, not DB-owned hot memory tables"
            );
            continue;
        }
        let mem_columns = columns_for_schema(&conn, MEMORY_SCHEMA, table);
        assert_eq!(
            mem_columns, main_columns,
            "{MEMORY_SCHEMA}.{table} must mirror main.{table}; memory schema is derived from the canonical disk schema"
        );
    }
}

#[test]
fn db_mem_flush_copies_each_row_once_and_empties_memory() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    create_memory_tables(&conn, &memory_uri_for_name("db_mem_flush_copies_each_row_once")).unwrap();
    let mut watermarks = initial_memory_flush_watermarks(&conn, ["net_events"]).expect("initial watermarks");
    let count = |schema: &str| -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {schema}.net_events"), [], |row| {
            row.get(0)
        })
        .unwrap()
    };
    let insert = |domain: &str| {
        conn.execute(
            "INSERT INTO mem.net_events (timestamp, domain, decision)
             VALUES ('2026-06-26T00:00:00Z', ?1, 'allowed')",
            [domain],
        )
        .unwrap();
    };

    insert("flush-one.example");
    insert("flush-two.example");
    let advanced = flush_memory_tables_to_disk(&conn, ["net_events"], &watermarks).expect("first flush");
    watermarks.extend(advanced);
    assert_eq!(
        (count("main"), count(MEMORY_SCHEMA)),
        (2, 0),
        "the first flush moves both rows"
    );

    insert("flush-three.example");
    let advanced = flush_memory_tables_to_disk(&conn, ["net_events"], &watermarks).expect("second flush");
    watermarks.extend(advanced);
    assert_eq!(
        (count("main"), count(MEMORY_SCHEMA)),
        (3, 0),
        "the second flush moves only the new row; the first two are not replayed"
    );

    let before_third = conn.total_changes();
    flush_memory_tables_to_disk(&conn, ["net_events"], &watermarks).expect("third flush");
    assert_eq!(
        conn.total_changes() - before_third,
        0,
        "a flush with nothing in memory writes nothing"
    );
}

#[test]
fn memory_sequences_start_above_every_id_the_disk_handed_out() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    conn.execute_batch(
        "INSERT INTO main.net_events (timestamp, domain, decision) VALUES ('t', 'a.example', 'allowed');
         UPDATE main.sqlite_sequence SET seq = 41 WHERE name = 'net_events';",
    )
    .unwrap();
    create_memory_tables(&conn, &memory_uri_for_name("memory_sequences_start_above_disk")).unwrap();
    seed_memory_sequences(&conn, hot_ledger_tables()).unwrap();
    conn.execute(
        "INSERT INTO mem.net_events (timestamp, domain, decision) VALUES ('t', 'b.example', 'allowed')",
        [],
    )
    .unwrap();
    assert_eq!(conn.last_insert_rowid(), 42);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM mem.net_events", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1,
        "seeding the sequence must copy no rows"
    );
}

#[test]
fn reader_pragmas_leave_the_connection_query_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    {
        let conn = Connection::open(&path).unwrap();
        apply_pragmas(&conn).unwrap();
        create_tables(&conn).unwrap();
    }

    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(&path, flags).unwrap();
    apply_reader_pragmas(&conn).unwrap();
    validate_ready_schema(&conn).expect("a query-only reader validates the disk schema");
    let error = conn
        .execute(
            "INSERT INTO net_events (timestamp, domain, decision) VALUES ('t', 'example.com', 'allowed')",
            [],
        )
        .expect_err("query_only must prevent writes through a reader");
    assert!(
        error.to_string().contains("readonly"),
        "query_only should make the reader worker effectively read-only: {error}"
    );
}

#[test]
fn apply_pragmas_succeeds() {
    let conn = Connection::open_in_memory().unwrap();
    apply_pragmas(&conn).unwrap();
}

#[test]
fn writer_pragmas_use_full_wal_durability() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Connection::open(dir.path().join("durable.db")).unwrap();
    apply_pragmas(&conn).unwrap();

    let journal: String = conn.query_row("PRAGMA journal_mode", [], |row| row.get(0)).unwrap();
    let synchronous: i64 = conn.query_row("PRAGMA synchronous", [], |row| row.get(0)).unwrap();
    assert_eq!(journal.to_ascii_lowercase(), "wal");
    assert_eq!(synchronous, 2, "SQLite FULL is pragma value 2");
    #[cfg(target_os = "macos")]
    assert_eq!(
        conn.query_row("PRAGMA fullfsync", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn writer_pragmas_enable_file_backed_mmap() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let conn = Connection::open(&path).unwrap();
    apply_pragmas(&conn).unwrap();

    let mmap_size: i64 = conn.query_row("PRAGMA mmap_size", [], |row| row.get(0)).unwrap();
    assert!(
        mmap_size >= SQLITE_MMAP_SIZE_BYTES,
        "writer connections must enable SQLite mmap inside the DB layer; got {mmap_size}"
    );
}

#[test]
fn create_tables_publishes_trace_columns() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    // Verify trace_id column exists by inserting a row with it.
    conn.execute(
        "INSERT INTO model_calls (timestamp, provider, method, path, trace_id)
             VALUES ('2024-01-01T00:00:00Z', 'test', 'POST', '/v1', 'trace_abc')",
        [],
    )
    .unwrap();
    let trace_id: String = conn
        .query_row(
            "SELECT trace_id FROM model_calls WHERE trace_id = 'trace_abc'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(trace_id, "trace_abc");
}

#[test]
fn create_tables_includes_fs_events() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    conn.execute(
        "INSERT INTO fs_events (timestamp, action, path, size)
             VALUES ('2026-01-01T00:00:00Z', 'created', 'project/app.js', 1234)",
        [],
    )
    .unwrap();
    let action: String = conn
        .query_row(
            "SELECT action FROM fs_events WHERE path = 'project/app.js'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(action, "created");
}

#[test]
fn create_tables_publishes_the_fs_events_table() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    conn.execute(
        "INSERT INTO fs_events (timestamp, action, path)
             VALUES ('2026-01-01T00:00:00Z', 'deleted', 'project/old.txt')",
        [],
    )
    .unwrap();
    let path: String = conn
        .query_row("SELECT path FROM fs_events WHERE action = 'deleted'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(path, "project/old.txt");
}

#[test]
fn create_tables_publishes_tool_call_origin_columns() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    // Verify origin/server/method columns exist by inserting one unified MCP-origin row.
    conn.execute(
        "INSERT INTO model_calls (timestamp, provider, method, path)
             VALUES ('2024-01-01T00:00:00Z', 'test', 'POST', '/v1')",
        [],
    )
    .unwrap();
    let mc_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO tool_calls (
                model_call_id, call_index, call_id, tool_name, origin, server_name, method
             ) VALUES (?1, 0, 'call_01', 'fetch_http', 'mcp', 'local', 'tools/call')",
        [mc_id],
    )
    .unwrap();
    let origin: String = conn
        .query_row("SELECT origin FROM tool_calls WHERE call_id = 'call_01'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(origin, "mcp");
}

/// An MCP tool call has no model call to hang from, and the table says so.
///
/// This was a `migrate` test: the old `tool_calls` had `model_call_id NOT
/// NULL`, and `rebuild_tool_calls_nullable_model_call` copied the table into
/// a nullable one. The declaration in `schema/ddl.rs` is nullable outright,
/// so what is left to hold is the invariant itself -- a tool call the agent
/// made directly through MCP is a real row, not an orphan to reject.
#[test]
fn an_mcp_tool_call_needs_no_model_call_to_belong_to() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();

    conn.execute(
        "INSERT INTO tool_calls (
                event_id, timestamp, model_call_id, provider, status, call_index,
                call_id, tool_name, arguments, response_preview, origin, server_name,
                method, request_id, decision, duration_ms
             ) VALUES (
                '012345abcdef', '2026-01-01T00:00:00Z', NULL, '', 'responded', 0,
                'mcp_01', 'fetch_http', '{\"url\":\"http://127.0.0.1\"}',
                'Status: 200 OK', 'mcp', 'local', 'tools/call', 'req-1', 'allowed', 7
             )",
        [],
    )
    .unwrap();

    let row: (Option<i64>, String, String) = conn
        .query_row(
            "SELECT model_call_id, origin, response_preview FROM tool_calls WHERE call_id = 'mcp_01'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(row.0, None);
    assert_eq!(row.1, "mcp");
    assert_eq!(row.2, "Status: 200 OK");
}

#[test]
fn create_tables_include_shared_credential_ref_columns() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();

    for table in [
        "net_events",
        "model_calls",
        "fs_events",
        "exec_events",
        "dns_events",
        "audit_events",
        "tool_calls",
        "tool_responses",
        "security_rule_events",
        "security_decision_events",
    ] {
        let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})")).unwrap();
        let cols: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert!(
            cols.iter().any(|col| col == "credential_ref"),
            "{table} missing top-level shared credential_ref column: {cols:?}"
        );
    }
}

#[test]
fn create_tables_include_shared_turn_id_columns() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();

    for table in [
        "net_events",
        "model_calls",
        "model_items",
        "tool_calls",
        "tool_responses",
        "event_body_blobs",
        "fs_events",
        "exec_events",
        "dns_events",
        "audit_events",
        "security_rule_events",
        "security_decision_events",
    ] {
        let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})")).unwrap();
        let cols: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert!(
            cols.iter().any(|col| col == "turn_id"),
            "{table} missing first-class turn_id column: {cols:?}"
        );
    }
}

#[test]
fn create_tables_include_shared_event_id_columns() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();

    for table in [
        "net_events",
        "model_calls",
        "fs_events",
        "exec_events",
        "dns_events",
        "audit_events",
        "substitution_events",
        "security_rule_events",
    ] {
        let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})")).unwrap();
        let cols: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert!(
            cols.iter().any(|col| col == "event_id"),
            "{table} missing shared event_id column: {cols:?}"
        );
    }
}

#[test]
fn model_calls_include_strict_protocol_column() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();

    let cols: Vec<String> = {
        let mut stmt = conn.prepare("PRAGMA table_info(model_calls)").unwrap();
        stmt.query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert!(
        cols.iter().any(|col| col == "protocol"),
        "model_calls must carry model wire protocol separately from provider: {cols:?}"
    );

    conn.execute(
        "INSERT INTO model_calls (timestamp, provider, protocol, method, path)
             VALUES ('2024-01-01T00:00:00Z', 'unknown', 'openai', 'POST', '/v1/chat/completions')",
        [],
    )
    .unwrap();
    let err = conn
        .execute(
            "INSERT INTO model_calls (timestamp, provider, protocol, method, path)
                 VALUES ('2024-01-01T00:00:00Z', 'unknown', 'madeup', 'POST', '/v1/chat/completions')",
            [],
        )
        .expect_err("unknown model wire protocols must be rejected");
    assert!(err.to_string().contains("CHECK"));
}

#[test]
fn create_tables_reject_raw_credential_ref_values() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();

    let err = conn
        .execute(
            "INSERT INTO net_events (
                    timestamp, domain, decision, credential_ref
                 ) VALUES (
                    '2026-01-01T00:00:00Z', 'api.github.com', 'allowed', 'ghp_raw_secret'
                 )",
            [],
        )
        .expect_err("raw credentials must not be accepted as credential_ref");
    assert!(
        err.to_string().contains("CHECK"),
        "expected CHECK constraint failure, got: {err}"
    );
}

#[test]
fn substitution_events_require_brokered_reference() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();

    conn.execute(
        "INSERT INTO substitution_events (
                timestamp, material_class, source, event_type,
                algorithm, substitution_ref, outcome
             ) VALUES (
                '2026-01-01T00:00:00Z', 'credential', 'http.authorization',
                'http.request', 'blake3',
                'credential:blake3:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef',
                'captured'
             )",
        [],
    )
    .unwrap();

    let err = conn
        .execute(
            "INSERT INTO substitution_events (
                    timestamp, material_class, source, algorithm,
                    substitution_ref, outcome
                 ) VALUES (
                    '2026-01-01T00:00:00Z', 'credential', 'http.authorization',
                    'blake3', 'Bearer raw-secret', 'captured'
                 )",
            [],
        )
        .expect_err("substitution_ref must be a brokered reference");
    assert!(
        err.to_string().contains("CHECK"),
        "expected CHECK constraint failure, got: {err}"
    );

    for outcome in ["substituted", "ignored"] {
        let err = conn
            .execute(
                "INSERT INTO substitution_events (
                        timestamp, material_class, source, event_type,
                        algorithm, substitution_ref, outcome
                     ) VALUES (
                        '2026-01-01T00:00:00Z', 'credential', 'http.authorization',
                        'http.request', 'blake3',
                        'credential:blake3:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef',
                        ?1
                     )",
                [outcome],
            )
            .expect_err("substitution_events outcome must be a closed broker verb");
        assert!(
            err.to_string().contains("CHECK"),
            "expected CHECK constraint failure for outcome {outcome}, got: {err}"
        );
    }
}

#[test]
fn create_tables_includes_security_rule_events_contract() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();

    conn.execute(
        "INSERT INTO security_rule_events (
                timestamp_unix_ms, event_id, event_type, rule_id,
                rule_action, detection_level, rule_json
             ) VALUES (
                1789000000000, 'abcdef123456', 'model.call',
                'openai_api_block', 'block', 'critical',
                '{\"name\":\"openai_api_block\",\"match\":\"model.provider == \\\"openai\\\"\"}'
             )",
        [],
    )
    .unwrap();

    let (event_id, rule_action, detection_level): (String, String, String) = conn
        .query_row(
            "SELECT event_id, rule_action, detection_level
                 FROM security_rule_events WHERE rule_id = 'openai_api_block'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(event_id, "abcdef123456");
    assert_eq!(rule_action, "block");
    assert_eq!(detection_level, "critical");
}

#[test]
fn create_tables_includes_security_ask_events_contract() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();

    conn.execute(
        "INSERT INTO security_ask_events (
                timestamp_unix_ms, ask_id, event_id, event_type, rule_id,
                rule_name, status, rule_json
             ) VALUES (
                1789000000000, 'abcdef123456', '111111abcdef',
                'http.request', 'profiles.rules.ask_openai', 'ask_openai',
                'pending', '{\"name\":\"ask_openai\"}'
             )",
        [],
    )
    .unwrap();

    let err = conn
        .execute(
            "INSERT INTO security_ask_events (
                    timestamp_unix_ms, ask_id, event_id, event_type, rule_id,
                    rule_name, status, rule_json
                 ) VALUES (
                    1789000000000, 'abcdef123457', '111111abcdeg',
                    'http.request', 'profiles.rules.ask_openai', 'ask_openai',
                    'maybe', '{}'
                 )",
            [],
        )
        .expect_err("ask status and ids must be strict");
    assert!(
        err.to_string().contains("CHECK"),
        "expected CHECK constraint failure, got: {err}"
    );
}

#[test]
fn security_rule_events_reject_unknown_rule_action() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();

    let err = conn
        .execute(
            "INSERT INTO security_rule_events (
                    timestamp_unix_ms, event_id, event_type, rule_id,
                    rule_action, rule_json
                 ) VALUES (
                    1789000000000, 'abcdef123456', 'model.call',
                    'old_detect', 'detect', '{}'
                 )",
            [],
        )
        .expect_err("detect is not a rule action");
    assert!(
        err.to_string().contains("CHECK"),
        "expected CHECK constraint failure, got: {err}"
    );
}

#[test]
fn security_rule_events_accept_rewrite_rule_action() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();

    conn.execute(
        "INSERT INTO security_rule_events (
                timestamp_unix_ms, event_id, event_type, rule_id,
                rule_action, rule_json
             ) VALUES (
                1789000000000, 'abcdef123456', 'model.call',
                'profiles.rules.redact_model', 'rewrite', '{}'
             )",
        [],
    )
    .expect("rewrite is a canonical stored action");
}

#[test]
fn security_decision_events_record_explicit_decisions_and_reject_magic_outcome() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();

    conn.execute(
        "INSERT INTO security_decision_events (
                timestamp_unix_ms, event_id, event_type, stage, actor,
                rule_id, plugin_id, previous_decision, requested_decision,
                effective_decision, reason
             ) VALUES (
                1789000000000, 'abcdef123456', 'file.import', 'rewrite',
                'dummy_pre_eicar', 'profiles.rules.scan_eicar', 'dummy_pre_eicar',
                'allow', 'block', 'block', 'EICAR test seed observed'
             )",
        [],
    )
    .expect("explicit decision transition must persist");

    let err = conn
        .execute(
            "INSERT INTO security_decision_events (
                    timestamp_unix_ms, event_id, event_type, stage, actor,
                    previous_decision, requested_decision, effective_decision
                 ) VALUES (
                    1789000000001, 'abcdef123457', 'file.import', 'rewrite',
                    'dummy_pre_eicar', 'allow', 'outcome', 'block'
                 )",
            [],
        )
        .expect_err("requested_decision must be an explicit decision, not magic outcome");
    assert!(
        err.to_string().contains("CHECK"),
        "expected CHECK constraint failure, got: {err}"
    );

    let err = conn
        .execute(
            "INSERT INTO security_decision_events (
                    timestamp_unix_ms, event_id, event_type, stage, actor,
                    previous_decision, requested_decision, effective_decision
                 ) VALUES (
                    1789000002, 'abcdef123458', 'file.import', 'mystery',
                    'dummy_pre_eicar', 'allow', 'block', 'block'
                 )",
            [],
        )
        .expect_err("stage must be canonical");
    assert!(
        err.to_string().contains("CHECK"),
        "expected CHECK constraint failure, got: {err}"
    );
}

#[test]
fn security_rule_events_reject_non_hex_event_id() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();

    let err = conn
        .execute(
            "INSERT INTO security_rule_events (
                    timestamp_unix_ms, event_id, event_type, rule_id,
                    rule_action, rule_json
                 ) VALUES (
                    1789000000000, 'evt_abc123', 'model.call',
                    'bad_event_id', 'allow', '{}'
                 )",
            [],
        )
        .expect_err("event_id must be 12 lowercase hex characters");
    assert!(
        err.to_string().contains("CHECK"),
        "expected CHECK constraint failure, got: {err}"
    );
}

#[test]
fn security_rule_events_reject_unknown_event_type() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();

    for event_type in ["dns.response", "model.request", "file.ingress"] {
        let err = conn
            .execute(
                "INSERT INTO security_rule_events (
                        timestamp_unix_ms, event_id, event_type, rule_id,
                        rule_action, rule_json
                     ) VALUES (
                        1789000000000, 'abcdef123456', ?1,
                        'stale_event_type', 'allow', '{}'
                     )",
                [event_type],
            )
            .expect_err("event_type must be a backed runtime event type");
        assert!(
            err.to_string().contains("CHECK"),
            "expected CHECK constraint failure for {event_type}, got: {err}"
        );
    }
}

#[test]
fn security_ask_events_reject_unknown_event_type() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();

    let err = conn
        .execute(
            "INSERT INTO security_ask_events (
                    timestamp_unix_ms, ask_id, event_id, event_type, rule_id,
                    rule_name, status, rule_json
                 ) VALUES (
                    1789000000000, 'abcdef123456', '111111abcdef',
                    'model.request', 'profiles.rules.ask_model', 'ask_model',
                    'pending', '{}'
                 )",
            [],
        )
        .expect_err("ask event_type must be a backed runtime event type");
    assert!(
        err.to_string().contains("CHECK"),
        "expected CHECK constraint failure, got: {err}"
    );
}

#[test]
fn security_rule_events_reject_unknown_detection_level() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();

    let err = conn
        .execute(
            "INSERT INTO security_rule_events (
                    timestamp_unix_ms, event_id, event_type, rule_id,
                    rule_action, detection_level, rule_json
                 ) VALUES (
                    1789000000000, 'abcdef123456', 'model.call',
                    'bad_level', 'allow', 'info', '{}'
                 )",
            [],
        )
        .expect_err("DB stores only canonical detection levels");
    assert!(
        err.to_string().contains("CHECK"),
        "expected CHECK constraint failure, got: {err}"
    );
}

#[test]
fn security_rule_events_reject_null_detection_level() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();

    let err = conn
        .execute(
            "INSERT INTO security_rule_events (
                    timestamp_unix_ms, event_id, event_type, rule_id,
                    rule_action, detection_level, rule_json
                 ) VALUES (
                    1789000000000, 'abcdef123456', 'model.call',
                    'ambiguous_level', 'allow', NULL, '{}'
                 )",
            [],
        )
        .expect_err("detection_level must be explicit none, not NULL");
    assert!(
        err.to_string().contains("NOT NULL") || err.to_string().contains("CHECK"),
        "expected NOT NULL/CHECK constraint failure, got: {err}"
    );
}

#[test]
fn security_rule_events_reject_non_json_forensic_payloads() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();

    let err = conn
        .execute(
            "INSERT INTO security_rule_events (
                    timestamp_unix_ms, event_id, event_type, rule_id,
                    rule_action, rule_json
                 ) VALUES (
                    1789000000000, 'abcdef123456', 'model.call',
                    'bad_payload', 'allow', 'not json'
                 )",
            [],
        )
        .expect_err("rule_json must be valid JSON");
    assert!(
        err.to_string().contains("CHECK"),
        "expected CHECK constraint failure, got: {err}"
    );
}

/// Writer pragmas (WAL + synchronous) must only be applied to read-write
/// connections. Read-only connections must use apply_reader_pragmas instead.
#[test]
fn reader_pragmas_work_on_readonly_connection() {
    // Create a file-backed DB first (writer sets WAL).
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    {
        let conn = Connection::open(&path).unwrap();
        apply_pragmas(&conn).unwrap();
        create_tables(&conn).unwrap();
    }

    // Open read-only -- apply_reader_pragmas must not fail.
    let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(&path, flags).unwrap();
    apply_reader_pragmas(&conn).unwrap();
}

#[test]
fn reader_pragmas_enable_mmap_before_query_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    {
        let conn = Connection::open(&path).unwrap();
        apply_pragmas(&conn).unwrap();
        create_tables(&conn).unwrap();
    }

    let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(&path, flags).unwrap();
    apply_reader_pragmas(&conn).unwrap();

    let mmap_size: i64 = conn.query_row("PRAGMA mmap_size", [], |row| row.get(0)).unwrap();
    assert!(
        mmap_size >= SQLITE_MMAP_SIZE_BYTES,
        "reader worker connections must enable SQLite mmap before query_only; got {mmap_size}"
    );

    let query_only: i64 = conn.query_row("PRAGMA query_only", [], |row| row.get(0)).unwrap();
    assert_eq!(query_only, 1, "reader worker must still be query-only");
}

#[test]
fn mmap_telemetry_records_budget_and_size_metrics() {
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};

    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _guard = metrics::set_default_local_recorder(&recorder);

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let conn = Connection::open(&path).unwrap();
    apply_pragmas(&conn).unwrap();
    create_tables(&conn).unwrap();
    record_sqlite_mmap_telemetry(&conn, &path, "writer", "test");

    let snapshot = snapshotter.snapshot().into_vec();
    assert!(snapshot.iter().any(|(key, _, _, value)| {
        key.key().name() == DB_SQLITE_MMAP_CONFIG_BYTES && matches!(value, DebugValue::Gauge(_))
    }));
    assert!(snapshot.iter().any(|(key, _, _, value)| {
        key.key().name() == DB_SQLITE_MMAP_EFFECTIVE_BYTES && matches!(value, DebugValue::Gauge(_))
    }));
    assert!(snapshot.iter().any(|(key, _, _, value)| {
        key.key().name() == DB_SQLITE_FILE_SIZE_BYTES && matches!(value, DebugValue::Gauge(_))
    }));
    assert!(snapshot.iter().any(|(key, _, _, value)| {
        key.key().name() == DB_SQLITE_WAL_SIZE_BYTES && matches!(value, DebugValue::Gauge(_))
    }));
    assert!(snapshot.iter().any(|(key, _, _, value)| {
        key.key().name() == DB_SQLITE_MMAP_COVERAGE_RATIO && matches!(value, DebugValue::Gauge(_))
    }));
    assert!(snapshot.iter().any(|(key, _, _, value)| {
        key.key().name() == DB_SQLITE_MMAP_BUDGET_CHECKS_TOTAL && matches!(value, DebugValue::Counter(1))
    }));
}

#[test]
fn create_tables_keeps_snapshots_out_of_session_db() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='snapshot_events'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        count, 0,
        "snapshots are host recovery state; session.db is the user/security activity ledger"
    );
}

#[test]
fn security_event_type_check_rejects_snapshot_event() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    let result = conn.execute(
        "INSERT INTO security_rule_events (
                timestamp_unix_ms, event_id, event_type, rule_id, rule_name,
                rule_action, detection_level, provider, rule_snapshot, event_payload
             ) VALUES (
                1, 'abcdef123456', 'snapshot.event', 'profiles.rules.snapshot',
                'snapshot', 'allow', 'none', 'profiles', '{}', '{}'
             )",
        [],
    );
    assert!(result.is_err(), "snapshot.event must not be a security-event type");
}

mod dns;

mod transport;
