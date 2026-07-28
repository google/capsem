use super::*;
use rusqlite::OpenFlags;

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

#[test]
fn create_tables_succeeds() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
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
    migrate(&conn);
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
fn fresh_create_schema_has_no_migration_only_columns() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    let before = READY_SCHEMA_COLUMNS
        .iter()
        .map(|(table, _)| (*table, columns_for_schema(&conn, "main", table)))
        .collect::<BTreeMap<_, _>>();

    migrate(&conn);

    for (table, columns_before_migrate) in before {
        assert_eq!(
            columns_before_migrate,
            columns_for_schema(&conn, "main", table),
            "fresh CREATE_SCHEMA must publish the final {table} shape; migrations are only for existing databases"
        );
    }
}

#[test]
fn fresh_schema_is_final_before_external_memory_rehydrate() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    create_memory_tables(
        &conn,
        &memory_uri_for_name("fresh_schema_is_final_before_external_memory_rehydrate"),
    )
    .unwrap();

    // Reproduce the production ordering window: an external reader mirrors
    // the freshly published schema before the writer runs legacy migrations.
    migrate(&conn);
    sync_memory_tables_from_disk(&conn, ["security_ask_events"])
        .expect("fresh canonical DDL must already match its post-migration shape");

    assert_eq!(
        columns_for_schema(&conn, MEMORY_SCHEMA, "security_ask_events"),
        columns_for_schema(&conn, "main", "security_ask_events"),
        "a fresh DB must not publish a pre-migration table shape to external readers"
    );
}

#[test]
fn db_mem_disk_ready_rejects_missing_memory_schema() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    migrate(&conn);

    let error = validate_ready_schema(&conn)
        .expect_err("ready() must fail if DB-owned memory tables were not created");
    assert!(
        error.contains("mem.net_events"),
        "missing memory schema must fail loudly instead of route projections hiding stale state: {error}"
    );
}

#[test]
fn db_mem_flush_uses_per_table_id_watermark() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    migrate(&conn);
    create_memory_tables(
        &conn,
        &memory_uri_for_name("db_mem_flush_uses_per_table_id_watermark"),
    )
    .unwrap();
    let mut watermarks =
        initial_memory_flush_watermarks(&conn, ["net_events"]).expect("initial watermarks");

    conn.execute(
        "INSERT INTO mem.net_events (timestamp, domain, decision)
             VALUES ('2026-06-26T00:00:00Z', 'flush-one.example', 'allowed')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO mem.net_events (timestamp, domain, decision)
             VALUES ('2026-06-26T00:00:01Z', 'flush-two.example', 'allowed')",
        [],
    )
    .unwrap();

    let before_first = conn.total_changes();
    let advanced =
        flush_memory_tables_to_disk(&conn, ["net_events"], &watermarks).expect("first flush");
    watermarks.extend(advanced);
    assert_eq!(
        conn.total_changes() - before_first,
        2,
        "first mem->disk flush should copy exactly the two new rows"
    );

    conn.execute(
        "INSERT INTO mem.net_events (timestamp, domain, decision)
             VALUES ('2026-06-26T00:00:02Z', 'flush-three.example', 'allowed')",
        [],
    )
    .unwrap();

    let before_second = conn.total_changes();
    let advanced =
        flush_memory_tables_to_disk(&conn, ["net_events"], &watermarks).expect("second flush");
    watermarks.extend(advanced);
    assert_eq!(
        conn.total_changes() - before_second,
        1,
        "second mem->disk flush must use the per-table id watermark instead of replaying the whole memory table"
    );

    let before_third = conn.total_changes();
    let advanced =
        flush_memory_tables_to_disk(&conn, ["net_events"], &watermarks).expect("third flush");
    watermarks.extend(advanced);
    assert_eq!(
        conn.total_changes() - before_third,
        0,
        "unchanged dirty-table flush must not rewrite already flushed rows"
    );
}

#[test]
fn db_mem_disk_memory_tables_work_before_query_only_guard() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    {
        let conn = Connection::open(&path).unwrap();
        apply_pragmas(&conn).unwrap();
        create_tables(&conn).unwrap();
        migrate(&conn);
    }

    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(&path, flags).unwrap();
    create_memory_tables(
        &conn,
        &memory_uri_for_name("db_mem_disk_memory_tables_work_before_query_only_guard"),
    )
    .unwrap();
    apply_reader_pragmas(&conn).unwrap();
    validate_ready_schema(&conn)
        .expect("query-only connection must still own its DB-local memory schema");
    let error = conn
        .execute("INSERT INTO mem.net_events (timestamp, domain, decision) VALUES ('t', 'example.com', 'allowed')", [])
        .expect_err("query_only must prevent writes after DB-owned memory setup");
    assert!(
        error.to_string().contains("readonly"),
        "query_only should make the reader worker effectively read-only after setup: {error}"
    );
}

#[test]
fn apply_pragmas_succeeds() {
    let conn = Connection::open_in_memory().unwrap();
    apply_pragmas(&conn).unwrap();
}

#[test]
fn writer_pragmas_enable_file_backed_mmap() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let conn = Connection::open(&path).unwrap();
    apply_pragmas(&conn).unwrap();

    let mmap_size: i64 = conn
        .query_row("PRAGMA mmap_size", [], |row| row.get(0))
        .unwrap();
    assert!(
        mmap_size >= SQLITE_MMAP_SIZE_BYTES,
        "writer connections must enable SQLite mmap inside the DB layer; got {mmap_size}"
    );
}

#[test]
fn migrate_trace_columns_idempotent() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    // Run twice -- second call must not error.
    migrate(&conn);
    migrate(&conn);
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
fn migrate_fs_events_idempotent() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    migrate(&conn);
    migrate(&conn);
    conn.execute(
        "INSERT INTO fs_events (timestamp, action, path)
             VALUES ('2026-01-01T00:00:00Z', 'deleted', 'project/old.txt')",
        [],
    )
    .unwrap();
    let path: String = conn
        .query_row(
            "SELECT path FROM fs_events WHERE action = 'deleted'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(path, "project/old.txt");
}

#[test]
fn migrate_tool_calls_origin_idempotent() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    migrate(&conn);
    migrate(&conn);
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
        .query_row(
            "SELECT origin FROM tool_calls WHERE call_id = 'call_01'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(origin, "mcp");
}

#[test]
fn migrate_tool_calls_allows_orphan_mcp_origin_rows() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE tool_calls (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                model_call_id INTEGER NOT NULL,
                call_index INTEGER NOT NULL,
                call_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                arguments TEXT
            );",
    )
    .unwrap();

    migrate(&conn);
    migrate(&conn);

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
fn migrate_event_body_blobs_accepts_tool_calls_source() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE event_body_blobs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                event_id TEXT NOT NULL CHECK (length(event_id) = 12),
                event_type TEXT NOT NULL CHECK (event_type IN ('http.request', 'model.call', 'mcp.tool_call')),
                source_table TEXT NOT NULL CHECK (source_table IN ('net_events', 'model_calls')),
                direction TEXT NOT NULL CHECK (direction IN ('request', 'response')),
                content_type TEXT,
                original_bytes INTEGER NOT NULL CHECK (original_bytes >= 0),
                stored_bytes INTEGER NOT NULL CHECK (stored_bytes >= 0 AND stored_bytes <= original_bytes),
                truncated INTEGER NOT NULL CHECK (truncated IN (0, 1)),
                body_hash TEXT NOT NULL CHECK (length(body_hash) = 71),
                body BLOB NOT NULL,
                trace_id TEXT,
                created_at TEXT NOT NULL,
                UNIQUE(event_id, source_table, direction)
            );",
    )
    .unwrap();

    migrate(&conn);
    migrate(&conn);

    conn.execute(
        "INSERT INTO event_body_blobs (
                event_id, event_type, source_table, direction, content_type,
                original_bytes, stored_bytes, truncated, body_hash, body,
                trace_id, created_at
             ) VALUES (
                '012345abcdef', 'mcp.tool_call', 'tool_calls', 'request',
                'application/json', 2, 2, 0,
                'blake3:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef',
                '{}', 'trace-1', '2026-01-01T00:00:00Z'
             )",
        [],
    )
    .unwrap();

    let source: String = conn
        .query_row(
            "SELECT source_table FROM event_body_blobs WHERE event_id = '012345abcdef'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(source, "tool_calls");
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
        let mut stmt = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap();
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
        let mut stmt = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap();
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
        let mut stmt = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap();
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
                rule_action, detection_level, rule_json, event_json
             ) VALUES (
                1789000000000, 'abcdef123456', 'model.call',
                'openai_api_block', 'block', 'critical',
                '{\"name\":\"openai_api_block\",\"match\":\"model.provider == \\\"openai\\\"\"}',
                '{\"common\":{\"event_type\":\"model.call\"},\"model\":{\"provider\":\"openai\"}}'
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
                rule_name, status, rule_json, event_json
             ) VALUES (
                1789000000000, 'abcdef123456', '111111abcdef',
                'http.request', 'profiles.rules.ask_openai', 'ask_openai',
                'pending', '{\"name\":\"ask_openai\"}',
                '{\"http\":{\"host\":\"api.openai.com\"}}'
             )",
        [],
    )
    .unwrap();

    let err = conn
        .execute(
            "INSERT INTO security_ask_events (
                    timestamp_unix_ms, ask_id, event_id, event_type, rule_id,
                    rule_name, status, rule_json, event_json
                 ) VALUES (
                    1789000000000, 'abcdef123457', '111111abcdeg',
                    'http.request', 'profiles.rules.ask_openai', 'ask_openai',
                    'maybe', '{}', '{}'
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
                    rule_action, rule_json, event_json
                 ) VALUES (
                    1789000000000, 'abcdef123456', 'model.call',
                    'old_detect', 'detect', '{}', '{}'
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
                rule_action, rule_json, event_json
             ) VALUES (
                1789000000000, 'abcdef123456', 'model.call',
                'profiles.rules.redact_model', 'rewrite', '{}', '{}'
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
                effective_decision, reason, event_json
             ) VALUES (
                1789000000000, 'abcdef123456', 'file.import', 'rewrite',
                'dummy_pre_eicar', 'profiles.rules.scan_eicar', 'dummy_pre_eicar',
                'allow', 'block', 'block', 'EICAR test seed observed', '{}'
             )",
        [],
    )
    .expect("explicit decision transition must persist");

    let err = conn
        .execute(
            "INSERT INTO security_decision_events (
                    timestamp_unix_ms, event_id, event_type, stage, actor,
                    previous_decision, requested_decision, effective_decision,
                    event_json
                 ) VALUES (
                    1789000000001, 'abcdef123457', 'file.import', 'rewrite',
                    'dummy_pre_eicar', 'allow', 'outcome', 'block', '{}'
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
                    previous_decision, requested_decision, effective_decision,
                    event_json
                 ) VALUES (
                    1789000002, 'abcdef123458', 'file.import', 'mystery',
                    'dummy_pre_eicar', 'allow', 'block', 'block', '{}'
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
                    rule_action, rule_json, event_json
                 ) VALUES (
                    1789000000000, 'evt_abc123', 'model.call',
                    'bad_event_id', 'allow', '{}', '{}'
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
                        rule_action, rule_json, event_json
                     ) VALUES (
                        1789000000000, 'abcdef123456', ?1,
                        'stale_event_type', 'allow', '{}', '{}'
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
                    rule_name, status, rule_json, event_json
                 ) VALUES (
                    1789000000000, 'abcdef123456', '111111abcdef',
                    'model.request', 'profiles.rules.ask_model', 'ask_model',
                    'pending', '{}', '{}'
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
                    rule_action, detection_level, rule_json, event_json
                 ) VALUES (
                    1789000000000, 'abcdef123456', 'model.call',
                    'bad_level', 'allow', 'info', '{}', '{}'
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
                    rule_action, detection_level, rule_json, event_json
                 ) VALUES (
                    1789000000000, 'abcdef123456', 'model.call',
                    'ambiguous_level', 'allow', NULL, '{}', '{}'
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
                    rule_action, rule_json, event_json
                 ) VALUES (
                    1789000000000, 'abcdef123456', 'model.call',
                    'bad_payload', 'allow', 'not json', '{}'
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
    let flags =
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
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

    let flags =
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(&path, flags).unwrap();
    apply_reader_pragmas(&conn).unwrap();

    let mmap_size: i64 = conn
        .query_row("PRAGMA mmap_size", [], |row| row.get(0))
        .unwrap();
    assert!(
        mmap_size >= SQLITE_MMAP_SIZE_BYTES,
        "reader worker connections must enable SQLite mmap before query_only; got {mmap_size}"
    );

    let query_only: i64 = conn
        .query_row("PRAGMA query_only", [], |row| row.get(0))
        .unwrap();
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
        key.key().name() == DB_SQLITE_MMAP_EFFECTIVE_BYTES
            && matches!(value, DebugValue::Gauge(_))
    }));
    assert!(snapshot.iter().any(|(key, _, _, value)| {
        key.key().name() == DB_SQLITE_FILE_SIZE_BYTES && matches!(value, DebugValue::Gauge(_))
    }));
    assert!(snapshot.iter().any(|(key, _, _, value)| {
        key.key().name() == DB_SQLITE_WAL_SIZE_BYTES && matches!(value, DebugValue::Gauge(_))
    }));
    assert!(snapshot.iter().any(|(key, _, _, value)| {
        key.key().name() == DB_SQLITE_MMAP_COVERAGE_RATIO
            && matches!(value, DebugValue::Gauge(_))
    }));
    assert!(snapshot.iter().any(|(key, _, _, value)| {
        key.key().name() == DB_SQLITE_MMAP_BUDGET_CHECKS_TOTAL
            && matches!(value, DebugValue::Counter(1))
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
    assert!(
        result.is_err(),
        "snapshot.event must not be a security-event type"
    );
}

#[test]
fn create_tables_includes_dns_events() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    conn.execute(
        "INSERT INTO dns_events (
                timestamp, qname, qtype, qclass, rcode, decision,
                policy_mode, policy_action, policy_rule, policy_reason
             )
             VALUES (
                '2026-01-01T00:00:00Z', 'anthropic.com', 1, 1, 0, 'allowed',
                'enforce', 'allow', 'policy.dns.allow_example', 'allowed by dns policy'
             )",
        [],
    )
    .unwrap();
    let (qname, policy_rule): (String, String) = conn
        .query_row(
            "SELECT qname, policy_rule FROM dns_events WHERE decision = 'allowed'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(qname, "anthropic.com");
    assert_eq!(policy_rule, "policy.dns.allow_example");
}

#[test]
fn migrate_dns_events_idempotent() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    // Run migrate twice -- second call must not error.
    migrate(&conn);
    migrate(&conn);
    // Verify dns_events table exists and accepts a row.
    conn.execute(
        "INSERT INTO dns_events (timestamp, qname, qtype, qclass, rcode, decision, trace_id)
             VALUES ('2026-01-01T00:00:00Z', 'pypi.org', 1, 1, 0, 'allowed', 'tr_abc')",
        [],
    )
    .unwrap();
    let trace: String = conn
        .query_row(
            "SELECT trace_id FROM dns_events WHERE qname = 'pypi.org'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(trace, "tr_abc");
}

#[test]
fn dns_events_has_indexes() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    for idx in [
        "idx_dns_events_timestamp",
        "idx_dns_events_qname",
        "idx_dns_events_trace_id",
        "idx_dns_events_decision",
        "idx_dns_events_policy_rule",
    ] {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name = ?1",
                [idx],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "missing index {idx}");
    }
}
