use rusqlite::Connection;

pub const CREATE_SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS net_events (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        timestamp TEXT NOT NULL,
        domain TEXT NOT NULL,
        port INTEGER DEFAULT 443,
        decision TEXT NOT NULL,
        process_name TEXT,
        pid INTEGER,
        method TEXT,
        path TEXT,
        query TEXT,
        status_code INTEGER,
        bytes_sent INTEGER DEFAULT 0,
        bytes_received INTEGER DEFAULT 0,
        duration_ms INTEGER DEFAULT 0,
        matched_rule TEXT,
        request_headers TEXT,
        response_headers TEXT,
        request_body_preview TEXT,
        response_body_preview TEXT,
        conn_type TEXT DEFAULT 'https',
        policy_mode TEXT,
        policy_action TEXT,
        policy_rule TEXT,
        policy_reason TEXT,
        trace_id TEXT
    );

    CREATE TABLE IF NOT EXISTS model_calls (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        timestamp TEXT NOT NULL,
        provider TEXT NOT NULL,
        model TEXT,
        process_name TEXT,
        pid INTEGER,
        method TEXT NOT NULL,
        path TEXT NOT NULL,
        stream INTEGER DEFAULT 0,
        system_prompt_preview TEXT,
        messages_count INTEGER DEFAULT 0,
        tools_count INTEGER DEFAULT 0,
        request_bytes INTEGER DEFAULT 0,
        request_body_preview TEXT,
        message_id TEXT,
        status_code INTEGER,
        text_content TEXT,
        thinking_content TEXT,
        stop_reason TEXT,
        input_tokens INTEGER,
        output_tokens INTEGER,
        duration_ms INTEGER DEFAULT 0,
        response_bytes INTEGER DEFAULT 0,
        estimated_cost_usd REAL DEFAULT 0,
        trace_id TEXT,
        usage_details TEXT
    );

    CREATE TABLE IF NOT EXISTS tool_calls (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        model_call_id INTEGER NOT NULL,
        call_index INTEGER NOT NULL,
        call_id TEXT NOT NULL,
        tool_name TEXT NOT NULL,
        arguments TEXT,
        origin TEXT NOT NULL DEFAULT 'native',
        mcp_call_id INTEGER,
        trace_id TEXT
    );

    CREATE TABLE IF NOT EXISTS tool_responses (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        model_call_id INTEGER NOT NULL,
        call_id TEXT NOT NULL,
        content_preview TEXT,
        is_error INTEGER DEFAULT 0,
        trace_id TEXT
    );

    CREATE INDEX IF NOT EXISTS idx_net_events_domain
        ON net_events(domain);
    CREATE INDEX IF NOT EXISTS idx_net_events_timestamp
        ON net_events(timestamp);
    CREATE INDEX IF NOT EXISTS idx_model_calls_provider_ts
        ON model_calls(provider, timestamp);
    CREATE INDEX IF NOT EXISTS idx_tool_calls_model_call
        ON tool_calls(model_call_id);
    CREATE INDEX IF NOT EXISTS idx_tool_responses_model_call
        ON tool_responses(model_call_id);
    CREATE INDEX IF NOT EXISTS idx_model_calls_trace_id
        ON model_calls(trace_id);

    CREATE TABLE IF NOT EXISTS mcp_calls (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        timestamp TEXT NOT NULL,
        server_name TEXT NOT NULL,
        method TEXT NOT NULL,
        tool_name TEXT,
        request_id TEXT,
        request_preview TEXT,
        response_preview TEXT,
        decision TEXT NOT NULL,
        duration_ms INTEGER DEFAULT 0,
        error_message TEXT,
        process_name TEXT,
        bytes_sent INTEGER DEFAULT 0,
        bytes_received INTEGER DEFAULT 0,
        policy_mode TEXT,
        policy_action TEXT,
        policy_rule TEXT,
        policy_reason TEXT,
        trace_id TEXT
    );

    CREATE INDEX IF NOT EXISTS idx_mcp_calls_server
        ON mcp_calls(server_name);
    CREATE INDEX IF NOT EXISTS idx_mcp_calls_timestamp
        ON mcp_calls(timestamp);
    CREATE INDEX IF NOT EXISTS idx_tool_calls_call_id
        ON tool_calls(call_id);
    CREATE INDEX IF NOT EXISTS idx_tool_responses_call_id
        ON tool_responses(call_id);

    CREATE TABLE IF NOT EXISTS fs_events (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        timestamp TEXT NOT NULL,
        action TEXT NOT NULL,
        path TEXT NOT NULL,
        size INTEGER,
        trace_id TEXT
    );

    CREATE INDEX IF NOT EXISTS idx_fs_events_timestamp
        ON fs_events(timestamp);
    CREATE INDEX IF NOT EXISTS idx_fs_events_path
        ON fs_events(path);

    CREATE TABLE IF NOT EXISTS snapshot_events (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        timestamp TEXT NOT NULL,
        slot INTEGER NOT NULL,
        origin TEXT NOT NULL,
        name TEXT,
        files_count INTEGER DEFAULT 0,
        start_fs_event_id INTEGER DEFAULT 0,
        stop_fs_event_id INTEGER DEFAULT 0,
        trace_id TEXT
    );
    CREATE INDEX IF NOT EXISTS idx_snapshot_events_timestamp
        ON snapshot_events(timestamp);

    CREATE TABLE IF NOT EXISTS session_identity (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        updated_at TEXT NOT NULL,
        vm_id TEXT NOT NULL,
        profile_id TEXT NOT NULL,
        user_id TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_session_identity_profile
        ON session_identity(profile_id);
    CREATE INDEX IF NOT EXISTS idx_session_identity_user
        ON session_identity(user_id);

    CREATE TABLE IF NOT EXISTS exec_events (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        timestamp TEXT NOT NULL,
        exec_id INTEGER NOT NULL,
        command TEXT NOT NULL,
        exit_code INTEGER,
        duration_ms INTEGER,
        stdout_preview TEXT,
        stderr_preview TEXT,
        stdout_bytes INTEGER DEFAULT 0,
        stderr_bytes INTEGER DEFAULT 0,
        source TEXT NOT NULL DEFAULT 'api',
        mcp_call_id INTEGER,
        trace_id TEXT,
        process_name TEXT,
        pid INTEGER
    );
    CREATE INDEX IF NOT EXISTS idx_exec_events_timestamp
        ON exec_events(timestamp);
    CREATE INDEX IF NOT EXISTS idx_exec_events_exec_id
        ON exec_events(exec_id);
    CREATE INDEX IF NOT EXISTS idx_exec_events_trace_id
        ON exec_events(trace_id);
    CREATE INDEX IF NOT EXISTS idx_exec_events_source
        ON exec_events(source);

    CREATE TABLE IF NOT EXISTS dns_events (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        timestamp TEXT NOT NULL,
        qname TEXT NOT NULL,
        qtype INTEGER NOT NULL,
        qclass INTEGER NOT NULL,
        rcode INTEGER NOT NULL,
        decision TEXT NOT NULL,
        matched_rule TEXT,
        source_proto TEXT,
        process_name TEXT,
        upstream_resolver_ms INTEGER DEFAULT 0,
        trace_id TEXT,
        policy_mode TEXT,
        policy_action TEXT,
        policy_rule TEXT,
        policy_reason TEXT
    );
    CREATE INDEX IF NOT EXISTS idx_dns_events_timestamp
        ON dns_events(timestamp);
    CREATE INDEX IF NOT EXISTS idx_dns_events_qname
        ON dns_events(qname);
    CREATE INDEX IF NOT EXISTS idx_dns_events_trace_id
        ON dns_events(trace_id);
    CREATE INDEX IF NOT EXISTS idx_dns_events_decision
        ON dns_events(decision);
    CREATE INDEX IF NOT EXISTS idx_dns_events_policy_rule
        ON dns_events(policy_rule);

    CREATE TABLE IF NOT EXISTS policy_hook_events (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        timestamp TEXT NOT NULL,
        endpoint_id TEXT NOT NULL,
        spec_version TEXT NOT NULL,
        spec_hash TEXT NOT NULL,
        decision_id TEXT,
        callback TEXT NOT NULL,
        decision TEXT,
        rule_id TEXT,
        reason TEXT,
        latency_ms INTEGER DEFAULT 0,
        status TEXT NOT NULL,
        error TEXT,
        fallback TEXT,
        audit_tags TEXT,
        trace_id TEXT,
        session_id TEXT
    );
    CREATE INDEX IF NOT EXISTS idx_policy_hook_events_timestamp
        ON policy_hook_events(timestamp);
    CREATE INDEX IF NOT EXISTS idx_policy_hook_events_endpoint
        ON policy_hook_events(endpoint_id);
    CREATE INDEX IF NOT EXISTS idx_policy_hook_events_trace_id
        ON policy_hook_events(trace_id);
    CREATE INDEX IF NOT EXISTS idx_policy_hook_events_decision_id
        ON policy_hook_events(decision_id);

    CREATE TABLE IF NOT EXISTS audit_events (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        timestamp TEXT NOT NULL,
        pid INTEGER NOT NULL,
        ppid INTEGER NOT NULL,
        uid INTEGER NOT NULL,
        exe TEXT NOT NULL,
        comm TEXT,
        argv TEXT NOT NULL,
        cwd TEXT,
        exit_code INTEGER,
        session_id INTEGER,
        tty TEXT,
        audit_id TEXT,
        exec_event_id INTEGER,
        parent_exe TEXT,
        trace_id TEXT
    );
    CREATE INDEX IF NOT EXISTS idx_audit_events_timestamp
        ON audit_events(timestamp);
    CREATE INDEX IF NOT EXISTS idx_audit_events_exe
        ON audit_events(exe);
    CREATE INDEX IF NOT EXISTS idx_audit_events_pid
        ON audit_events(pid);
    CREATE INDEX IF NOT EXISTS idx_audit_events_ppid
        ON audit_events(ppid);
";

/// Create all tables and indexes on the given connection.
pub fn create_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(CREATE_SCHEMA)
}

/// Apply write-mode pragmas: WAL journal + relaxed synchronous.
/// Only call on read-write connections (the writer).
pub fn apply_pragmas(conn: &Connection) -> rusqlite::Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(())
}

/// Migrate existing databases to add new columns/tables.
/// Idempotent: safe to call on databases that already have the changes.
pub fn migrate(conn: &Connection) {
    let _ = conn.execute("ALTER TABLE model_calls ADD COLUMN trace_id TEXT", []);
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_model_calls_trace_id ON model_calls(trace_id)",
        [],
    );
    // Add mcp_calls table if not present (for DBs created before this feature).
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS mcp_calls (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp TEXT NOT NULL,
            server_name TEXT NOT NULL,
            method TEXT NOT NULL,
            tool_name TEXT,
            request_id TEXT,
            request_preview TEXT,
            response_preview TEXT,
            decision TEXT NOT NULL,
            duration_ms INTEGER DEFAULT 0,
            error_message TEXT,
            process_name TEXT,
            bytes_sent INTEGER DEFAULT 0,
            bytes_received INTEGER DEFAULT 0,
            policy_mode TEXT,
            policy_action TEXT,
            policy_rule TEXT,
            policy_reason TEXT,
            trace_id TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_mcp_calls_server ON mcp_calls(server_name);
        CREATE INDEX IF NOT EXISTS idx_mcp_calls_timestamp ON mcp_calls(timestamp);",
    );
    // Replace cache_read_tokens with usage_details TEXT column.
    // SQLite doesn't support DROP COLUMN before 3.35, so just add the new one.
    let _ = conn.execute("ALTER TABLE model_calls ADD COLUMN usage_details TEXT", []);
    // Add origin + mcp_call_id columns to tool_calls (for DBs created before this feature).
    let _ = conn.execute(
        "ALTER TABLE tool_calls ADD COLUMN origin TEXT NOT NULL DEFAULT 'native'",
        [],
    );
    let _ = conn.execute("ALTER TABLE tool_calls ADD COLUMN mcp_call_id INTEGER", []);
    // Add bytes_sent/bytes_received to mcp_calls (for DBs created before this feature).
    let _ = conn.execute(
        "ALTER TABLE mcp_calls ADD COLUMN bytes_sent INTEGER DEFAULT 0",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE mcp_calls ADD COLUMN bytes_received INTEGER DEFAULT 0",
        [],
    );
    // Add policy decision metadata to mcp_calls (for DBs created before T2).
    let _ = conn.execute("ALTER TABLE mcp_calls ADD COLUMN policy_mode TEXT", []);
    let _ = conn.execute("ALTER TABLE mcp_calls ADD COLUMN policy_action TEXT", []);
    let _ = conn.execute("ALTER TABLE mcp_calls ADD COLUMN policy_rule TEXT", []);
    let _ = conn.execute("ALTER TABLE mcp_calls ADD COLUMN policy_reason TEXT", []);
    // Add policy decision metadata to net_events for Policy V2 HTTP/DNS audit.
    let _ = conn.execute("ALTER TABLE net_events ADD COLUMN policy_mode TEXT", []);
    let _ = conn.execute("ALTER TABLE net_events ADD COLUMN policy_action TEXT", []);
    let _ = conn.execute("ALTER TABLE net_events ADD COLUMN policy_rule TEXT", []);
    let _ = conn.execute("ALTER TABLE net_events ADD COLUMN policy_reason TEXT", []);
    // Add indexes for tool_calls/tool_responses call_id lookups.
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_tool_calls_call_id ON tool_calls(call_id)",
        [],
    );
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_tool_responses_call_id ON tool_responses(call_id)",
        [],
    );
    // Add fs_events table if not present (for DBs created before this feature).
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS fs_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp TEXT NOT NULL,
            action TEXT NOT NULL,
            path TEXT NOT NULL,
            size INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_fs_events_timestamp ON fs_events(timestamp);
        CREATE INDEX IF NOT EXISTS idx_fs_events_path ON fs_events(path);",
    );
    // Add snapshot_events table if not present (for DBs created before this feature).
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS snapshot_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp TEXT NOT NULL,
            slot INTEGER NOT NULL,
            origin TEXT NOT NULL,
            name TEXT,
            files_count INTEGER DEFAULT 0,
            start_fs_event_id INTEGER DEFAULT 0,
            stop_fs_event_id INTEGER DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_snapshot_events_timestamp ON snapshot_events(timestamp);",
    );
    // Add exec_events table if not present (for DBs created before this feature).
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS exec_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp TEXT NOT NULL,
            exec_id INTEGER NOT NULL,
            command TEXT NOT NULL,
            exit_code INTEGER,
            duration_ms INTEGER,
            stdout_preview TEXT,
            stderr_preview TEXT,
            stdout_bytes INTEGER DEFAULT 0,
            stderr_bytes INTEGER DEFAULT 0,
            source TEXT NOT NULL DEFAULT 'api',
            mcp_call_id INTEGER,
            trace_id TEXT,
            process_name TEXT,
            pid INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_exec_events_timestamp ON exec_events(timestamp);
        CREATE INDEX IF NOT EXISTS idx_exec_events_exec_id ON exec_events(exec_id);
        CREATE INDEX IF NOT EXISTS idx_exec_events_trace_id ON exec_events(trace_id);
        CREATE INDEX IF NOT EXISTS idx_exec_events_source ON exec_events(source);",
    );
    // S07a: one durable identity row per session DB. This keeps event writes
    // lean while making VM/profile/user identity available to telemetry
    // exports, detail/status paths, and support bundles.
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS session_identity (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            updated_at TEXT NOT NULL,
            vm_id TEXT NOT NULL,
            profile_id TEXT NOT NULL,
            user_id TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_session_identity_profile
            ON session_identity(profile_id);
        CREATE INDEX IF NOT EXISTS idx_session_identity_user
            ON session_identity(user_id);",
    );
    // T3.3: Add dns_events table if not present (for DBs created before
    // T3 landed). The host-side DNS proxy writes one row per resolved
    // query; trace_id correlates back to the same agent action that
    // emitted the corresponding net_events / model_calls rows.
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS dns_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp TEXT NOT NULL,
            qname TEXT NOT NULL,
            qtype INTEGER NOT NULL,
            qclass INTEGER NOT NULL,
            rcode INTEGER NOT NULL,
            decision TEXT NOT NULL,
            matched_rule TEXT,
            source_proto TEXT,
            process_name TEXT,
            upstream_resolver_ms INTEGER DEFAULT 0,
            trace_id TEXT,
            policy_mode TEXT,
            policy_action TEXT,
            policy_rule TEXT,
            policy_reason TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_dns_events_timestamp ON dns_events(timestamp);
        CREATE INDEX IF NOT EXISTS idx_dns_events_qname ON dns_events(qname);
        CREATE INDEX IF NOT EXISTS idx_dns_events_trace_id ON dns_events(trace_id);
        CREATE INDEX IF NOT EXISTS idx_dns_events_decision ON dns_events(decision);
        CREATE INDEX IF NOT EXISTS idx_dns_events_policy_rule ON dns_events(policy_rule);",
    );
    let _ = conn.execute("ALTER TABLE dns_events ADD COLUMN policy_mode TEXT", []);
    let _ = conn.execute("ALTER TABLE dns_events ADD COLUMN policy_action TEXT", []);
    let _ = conn.execute("ALTER TABLE dns_events ADD COLUMN policy_rule TEXT", []);
    let _ = conn.execute("ALTER TABLE dns_events ADD COLUMN policy_reason TEXT", []);
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_dns_events_policy_rule ON dns_events(policy_rule)",
        [],
    );

    // Add policy_hook_events table if not present (for DBs created before
    // external Policy Hook runtime support). These rows record every hook
    // decision attempt, including fail-closed schema/transport errors.
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS policy_hook_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp TEXT NOT NULL,
            endpoint_id TEXT NOT NULL,
            spec_version TEXT NOT NULL,
            spec_hash TEXT NOT NULL,
            decision_id TEXT,
            callback TEXT NOT NULL,
            decision TEXT,
            rule_id TEXT,
            reason TEXT,
            latency_ms INTEGER DEFAULT 0,
            status TEXT NOT NULL,
            error TEXT,
            fallback TEXT,
            audit_tags TEXT,
            trace_id TEXT,
            session_id TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_policy_hook_events_timestamp
            ON policy_hook_events(timestamp);
        CREATE INDEX IF NOT EXISTS idx_policy_hook_events_endpoint
            ON policy_hook_events(endpoint_id);
        CREATE INDEX IF NOT EXISTS idx_policy_hook_events_trace_id
            ON policy_hook_events(trace_id);
        CREATE INDEX IF NOT EXISTS idx_policy_hook_events_decision_id
            ON policy_hook_events(decision_id);",
    );

    // Add audit_events table if not present (for DBs created before this feature).
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS audit_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp TEXT NOT NULL,
            pid INTEGER NOT NULL,
            ppid INTEGER NOT NULL,
            uid INTEGER NOT NULL,
            exe TEXT NOT NULL,
            comm TEXT,
            argv TEXT NOT NULL,
            cwd TEXT,
            exit_code INTEGER,
            session_id INTEGER,
            tty TEXT,
            audit_id TEXT,
            exec_event_id INTEGER,
            parent_exe TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_audit_events_timestamp ON audit_events(timestamp);
        CREATE INDEX IF NOT EXISTS idx_audit_events_exe ON audit_events(exe);
        CREATE INDEX IF NOT EXISTS idx_audit_events_pid ON audit_events(pid);
        CREATE INDEX IF NOT EXISTS idx_audit_events_ppid ON audit_events(ppid);",
    );
    let _ = conn.execute("ALTER TABLE audit_events ADD COLUMN exit_code INTEGER", []);

    // W6: trace_id everywhere. Adding the column to the seven tables that
    // didn't already have it lets `capsem_timeline --trace_id <X>` join
    // every event class for one logical user action. NULL for rows that
    // pre-date W4's trace propagation; downstream queries handle that
    // gracefully (`WHERE trace_id = ? OR trace_id IS NULL`).
    for tbl in [
        "mcp_calls",
        "net_events",
        "fs_events",
        "snapshot_events",
        "tool_calls",
        "tool_responses",
        "audit_events",
    ] {
        let _ = conn.execute(&format!("ALTER TABLE {tbl} ADD COLUMN trace_id TEXT"), []);
        let _ = conn.execute(
            &format!("CREATE INDEX IF NOT EXISTS idx_{tbl}_trace_id ON {tbl}(trace_id)"),
            [],
        );
    }
}

/// Apply read-safe pragmas for read-only connections.
/// WAL mode is inherited from the file; no write pragmas needed.
pub fn apply_reader_pragmas(conn: &Connection) -> rusqlite::Result<()> {
    conn.pragma_update(None, "query_only", "ON")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn apply_pragmas_succeeds() {
        let conn = Connection::open_in_memory().unwrap();
        apply_pragmas(&conn).unwrap();
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
    fn migrate_mcp_calls_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();
        migrate(&conn);
        migrate(&conn);
        // Verify mcp_calls table exists.
        conn.execute(
            "INSERT INTO mcp_calls (timestamp, server_name, method, decision)
             VALUES ('2024-01-01T00:00:00Z', 'github', 'tools/list', 'allowed')",
            [],
        )
        .unwrap();
        let server: String = conn
            .query_row(
                "SELECT server_name FROM mcp_calls WHERE server_name = 'github'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(server, "github");
    }

    #[test]
    fn migrate_policy_hook_events_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();
        migrate(&conn);
        migrate(&conn);
        conn.execute(
            "INSERT INTO policy_hook_events (
                timestamp, endpoint_id, spec_version, spec_hash, callback, status
             )
             VALUES (
                '2026-01-01T00:00:00Z', 'fixture', 'policy-hook/v0', 'hash',
                'http.request', 'allowed'
             )",
            [],
        )
        .unwrap();
        let endpoint: String = conn
            .query_row(
                "SELECT endpoint_id FROM policy_hook_events WHERE callback = 'http.request'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(endpoint, "fixture");
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
        // Verify origin and mcp_call_id columns exist by inserting a row.
        conn.execute(
            "INSERT INTO model_calls (timestamp, provider, method, path)
             VALUES ('2024-01-01T00:00:00Z', 'test', 'POST', '/v1')",
            [],
        )
        .unwrap();
        let mc_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO tool_calls (model_call_id, call_index, call_id, tool_name, origin, mcp_call_id)
             VALUES (?1, 0, 'call_01', 'fetch_http', 'mcp', NULL)",
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
    fn migrate_mcp_calls_bytes_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();
        migrate(&conn);
        migrate(&conn);
        // Verify bytes_sent/bytes_received columns exist.
        conn.execute(
            "INSERT INTO mcp_calls (timestamp, server_name, method, decision, bytes_sent, bytes_received)
             VALUES ('2026-01-01T00:00:00Z', 'test', 'tools/call', 'allowed', 1024, 2048)",
            [],
        )
        .unwrap();
        let (sent, recv): (i64, i64) = conn
            .query_row(
                "SELECT bytes_sent, bytes_received FROM mcp_calls WHERE server_name = 'test'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(sent, 1024);
        assert_eq!(recv, 2048);
    }

    #[test]
    fn migrate_mcp_calls_policy_fields_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();
        migrate(&conn);
        migrate(&conn);
        conn.execute(
            "INSERT INTO mcp_calls (
                timestamp, server_name, method, decision,
                policy_mode, policy_action, policy_rule, policy_reason
             )
             VALUES (
                '2026-01-01T00:00:00Z', 'github', 'tools/call', 'allowed',
                'audit_only', 'deny', 'mcp.tool.github__delete_repo', 'local policy block'
             )",
            [],
        )
        .unwrap();
        let (mode, action, rule, reason): (String, String, String, String) = conn
            .query_row(
                "SELECT policy_mode, policy_action, policy_rule, policy_reason
                 FROM mcp_calls WHERE server_name = 'github'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(mode, "audit_only");
        assert_eq!(action, "deny");
        assert_eq!(rule, "mcp.tool.github__delete_repo");
        assert_eq!(reason, "local policy block");
    }

    #[test]
    fn migrate_legacy_pre_policy_db_adds_current_tables_and_columns() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE net_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT NOT NULL,
                domain TEXT NOT NULL,
                decision TEXT NOT NULL
            );
            CREATE TABLE model_calls (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT NOT NULL,
                provider TEXT NOT NULL,
                method TEXT NOT NULL,
                path TEXT NOT NULL
            );
            CREATE TABLE tool_calls (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                model_call_id INTEGER NOT NULL,
                call_index INTEGER NOT NULL,
                call_id TEXT NOT NULL,
                tool_name TEXT NOT NULL
            );
            CREATE TABLE tool_responses (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                model_call_id INTEGER NOT NULL,
                call_id TEXT NOT NULL
            );
            CREATE TABLE mcp_calls (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT NOT NULL,
                server_name TEXT NOT NULL,
                method TEXT NOT NULL,
                decision TEXT NOT NULL
            );
            CREATE TABLE fs_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT NOT NULL,
                action TEXT NOT NULL,
                path TEXT NOT NULL,
                size INTEGER
            );",
        )
        .unwrap();

        migrate(&conn);
        migrate(&conn);

        for table in [
            "dns_events",
            "exec_events",
            "snapshot_events",
            "audit_events",
            "policy_hook_events",
        ] {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name = ?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "missing migrated table {table}");
        }

        for (table, column) in [
            ("net_events", "policy_action"),
            ("mcp_calls", "policy_reason"),
            ("dns_events", "policy_rule"),
            ("tool_calls", "mcp_call_id"),
            ("tool_responses", "trace_id"),
            ("fs_events", "trace_id"),
            ("snapshot_events", "trace_id"),
            ("audit_events", "exit_code"),
            ("audit_events", "trace_id"),
            ("policy_hook_events", "fallback"),
        ] {
            let count: i64 = conn
                .query_row(
                    &format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name = ?1"),
                    [column],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "{table} missing migrated column {column}");
        }

        conn.execute(
            "INSERT INTO dns_events (
                timestamp, qname, qtype, qclass, rcode, decision,
                policy_mode, policy_action, policy_rule, policy_reason, trace_id
             )
             VALUES (
                '2026-05-10T00:00:00Z', 'blocked.example', 1, 1, 5, 'denied',
                'v2', 'block', 'policy.dns.block_example', 'fixture', 'trace_legacy'
             )",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO policy_hook_events (
                timestamp, endpoint_id, spec_version, spec_hash, callback,
                status, fallback, error, trace_id
             )
             VALUES (
                '2026-05-10T00:00:01Z', 'legacy-hook', 'policy-hook/v0',
                'sha256:legacy', 'dns.request', 'error', 'fail_closed',
                'schema violation', 'trace_legacy'
             )",
            [],
        )
        .unwrap();
    }

    #[test]
    fn create_tables_includes_snapshot_events() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();
        conn.execute(
            "INSERT INTO snapshot_events (timestamp, slot, origin, name, files_count, start_fs_event_id, stop_fs_event_id)
             VALUES ('2026-01-01T00:00:00Z', 0, 'auto', NULL, 14, 0, 5)",
            [],
        )
        .unwrap();
        let (slot, origin, files_count, start_id, stop_id): (i64, String, i64, i64, i64) = conn
            .query_row(
                "SELECT slot, origin, files_count, start_fs_event_id, stop_fs_event_id FROM snapshot_events",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .unwrap();
        assert_eq!(slot, 0);
        assert_eq!(origin, "auto");
        assert_eq!(files_count, 14);
        assert_eq!(start_id, 0);
        assert_eq!(stop_id, 5);
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

    #[test]
    fn migrate_snapshot_events_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();
        migrate(&conn);
        migrate(&conn);
        conn.execute(
            "INSERT INTO snapshot_events (timestamp, slot, origin, files_count, start_fs_event_id, stop_fs_event_id)
             VALUES ('2026-01-01T00:00:00Z', 5, 'manual', 20, 10, 25)",
            [],
        )
        .unwrap();
        let origin: String = conn
            .query_row(
                "SELECT origin FROM snapshot_events WHERE slot = 5",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(origin, "manual");
    }

    #[test]
    fn migrate_session_identity_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();
        migrate(&conn);
        migrate(&conn);
        conn.execute(
            "INSERT INTO session_identity (id, updated_at, vm_id, profile_id, user_id)
             VALUES (1, '2026-05-18T00:00:00Z', 'vm-1', 'everyday-work', 'elie')",
            [],
        )
        .unwrap();
        let identity: (String, String, String) = conn
            .query_row(
                "SELECT vm_id, profile_id, user_id FROM session_identity WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            identity,
            (
                "vm-1".to_string(),
                "everyday-work".to_string(),
                "elie".to_string()
            )
        );
    }
}
