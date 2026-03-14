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
        conn_type TEXT DEFAULT 'https'
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
        mcp_call_id INTEGER
    );

    CREATE TABLE IF NOT EXISTS tool_responses (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        model_call_id INTEGER NOT NULL,
        call_id TEXT NOT NULL,
        content_preview TEXT,
        is_error INTEGER DEFAULT 0
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
        bytes_received INTEGER DEFAULT 0
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
        size INTEGER
    );

    CREATE INDEX IF NOT EXISTS idx_fs_events_timestamp
        ON fs_events(timestamp);
    CREATE INDEX IF NOT EXISTS idx_fs_events_path
        ON fs_events(path);
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
            process_name TEXT
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
    let _ = conn.execute(
        "ALTER TABLE tool_calls ADD COLUMN mcp_call_id INTEGER",
        [],
    );
    // Add bytes_sent/bytes_received to mcp_calls (for DBs created before this feature).
    let _ = conn.execute(
        "ALTER TABLE mcp_calls ADD COLUMN bytes_sent INTEGER DEFAULT 0",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE mcp_calls ADD COLUMN bytes_received INTEGER DEFAULT 0",
        [],
    );
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
        let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
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
}
