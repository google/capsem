use std::path::Path;

use rusqlite::{params, Connection};

use super::types::*;

/// Session index database wrapping `~/.capsem/sessions/main.db`.
pub struct SessionIndex {
    pub(crate) conn: Connection,
}

/// Current schema version for main.db.
pub(super) const SCHEMA_VERSION: u32 = 4;

pub(super) const SESSION_SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS sessions (
        id TEXT PRIMARY KEY,
        mode TEXT NOT NULL,
        command TEXT,
        status TEXT NOT NULL DEFAULT 'running',
        created_at TEXT NOT NULL,
        stopped_at TEXT,
        scratch_disk_size_gb INTEGER NOT NULL DEFAULT 16,
        ram_bytes INTEGER NOT NULL DEFAULT 4294967296,
        total_requests INTEGER NOT NULL DEFAULT 0,
        allowed_requests INTEGER NOT NULL DEFAULT 0,
        denied_requests INTEGER NOT NULL DEFAULT 0,
        total_input_tokens INTEGER NOT NULL DEFAULT 0,
        total_output_tokens INTEGER NOT NULL DEFAULT 0,
        total_estimated_cost REAL NOT NULL DEFAULT 0.0,
        total_tool_calls INTEGER NOT NULL DEFAULT 0,
        total_mcp_calls INTEGER NOT NULL DEFAULT 0,
        total_file_events INTEGER NOT NULL DEFAULT 0,
        compressed_size_bytes INTEGER,
        vacuumed_at TEXT,
        storage_mode TEXT NOT NULL DEFAULT 'block',
        rootfs_hash TEXT,
        rootfs_version TEXT
    );
    CREATE INDEX IF NOT EXISTS idx_sessions_created
        ON sessions(created_at);
    CREATE INDEX IF NOT EXISTS idx_sessions_status
        ON sessions(status);

    CREATE TABLE IF NOT EXISTS ai_usage (
        session_id    TEXT NOT NULL,
        provider      TEXT NOT NULL,
        call_count    INTEGER NOT NULL DEFAULT 0,
        input_tokens  INTEGER NOT NULL DEFAULT 0,
        output_tokens INTEGER NOT NULL DEFAULT 0,
        estimated_cost REAL NOT NULL DEFAULT 0.0,
        total_duration_ms INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (session_id, provider)
    );

    CREATE TABLE IF NOT EXISTS tool_usage (
        session_id    TEXT NOT NULL,
        tool_name     TEXT NOT NULL,
        call_count    INTEGER NOT NULL DEFAULT 0,
        total_bytes   INTEGER NOT NULL DEFAULT 0,
        total_duration_ms INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (session_id, tool_name)
    );

    CREATE TABLE IF NOT EXISTS mcp_usage (
        session_id    TEXT NOT NULL,
        tool_name     TEXT NOT NULL,
        server_name   TEXT NOT NULL,
        call_count    INTEGER NOT NULL DEFAULT 0,
        total_bytes   INTEGER NOT NULL DEFAULT 0,
        total_duration_ms INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (session_id, tool_name)
    );
";

impl SessionIndex {
    /// Open (or create) the session index at the given path.
    /// Handles schema migration: if the DB is at an older version, drops
    /// all tables and recreates at the current version.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::ensure_schema(&conn)?;
        Ok(Self { conn })
    }

    /// Open an in-memory database (for testing).
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::ensure_schema(&conn)?;
        Ok(Self { conn })
    }

    /// Check user_version and migrate if needed.
    pub(crate) fn ensure_schema(conn: &Connection) -> rusqlite::Result<()> {
        let version: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version == 3 {
            // Additive migration v3->v4: add VirtioFS storage columns.
            conn.execute_batch(
                "ALTER TABLE sessions ADD COLUMN storage_mode TEXT NOT NULL DEFAULT 'block';
                 ALTER TABLE sessions ADD COLUMN rootfs_hash TEXT;
                 ALTER TABLE sessions ADD COLUMN rootfs_version TEXT;"
            )?;
            conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        } else if version == 2 {
            // Additive migration v2->v3->v4: add vacuum + VirtioFS columns.
            conn.execute_batch(
                "ALTER TABLE sessions ADD COLUMN compressed_size_bytes INTEGER;
                 ALTER TABLE sessions ADD COLUMN vacuumed_at TEXT;
                 ALTER TABLE sessions ADD COLUMN storage_mode TEXT NOT NULL DEFAULT 'block';
                 ALTER TABLE sessions ADD COLUMN rootfs_hash TEXT;
                 ALTER TABLE sessions ADD COLUMN rootfs_version TEXT;"
            )?;
            conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        } else if version < 2 {
            // Old schema -- drop and recreate.
            conn.execute_batch(
                "DROP TABLE IF EXISTS sessions;
                 DROP TABLE IF EXISTS ai_usage;
                 DROP TABLE IF EXISTS tool_usage;
                 DROP TABLE IF EXISTS mcp_usage;"
            )?;
            conn.execute_batch(SESSION_SCHEMA)?;
            conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        } else {
            // Already at current version -- just ensure tables exist.
            conn.execute_batch(SESSION_SCHEMA)?;
        }
        Ok(())
    }

    /// Insert a new session record.
    pub fn create_session(&self, record: &SessionRecord) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO sessions (id, mode, command, status, created_at, stopped_at,
                scratch_disk_size_gb, ram_bytes, total_requests, allowed_requests, denied_requests,
                total_input_tokens, total_output_tokens, total_estimated_cost,
                total_tool_calls, total_mcp_calls, total_file_events,
                compressed_size_bytes, vacuumed_at,
                storage_mode, rootfs_hash, rootfs_version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22)",
            params![
                record.id,
                record.mode,
                record.command,
                record.status,
                record.created_at,
                record.stopped_at,
                record.scratch_disk_size_gb as i64,
                record.ram_bytes as i64,
                record.total_requests as i64,
                record.allowed_requests as i64,
                record.denied_requests as i64,
                record.total_input_tokens as i64,
                record.total_output_tokens as i64,
                record.total_estimated_cost,
                record.total_tool_calls as i64,
                record.total_mcp_calls as i64,
                record.total_file_events as i64,
                record.compressed_size_bytes.map(|v| v as i64),
                record.vacuumed_at,
                record.storage_mode,
                record.rootfs_hash,
                record.rootfs_version,
            ],
        )?;
        Ok(())
    }

    /// Update session status and optionally set stopped_at.
    pub fn update_status(
        &self,
        id: &str,
        status: &str,
        stopped_at: Option<&str>,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE sessions SET status = ?1, stopped_at = ?2 WHERE id = ?3",
            params![status, stopped_at, id],
        )?;
        Ok(())
    }

    /// Update request counts for a session.
    pub fn update_request_counts(
        &self,
        id: &str,
        total: u64,
        allowed: u64,
        denied: u64,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE sessions SET total_requests = ?1, allowed_requests = ?2, denied_requests = ?3
             WHERE id = ?4",
            params![total as i64, allowed as i64, denied as i64, id],
        )?;
        Ok(())
    }

    /// Mark all "running" sessions as "crashed". Returns count of affected rows.
    pub fn mark_running_as_crashed(&self) -> rusqlite::Result<usize> {
        let count = self.conn.execute(
            "UPDATE sessions SET status = 'crashed' WHERE status = 'running'",
            [],
        )?;
        Ok(count)
    }

    /// Shared column list for SELECT queries on sessions.
    const SESSION_COLUMNS: &str =
        "id, mode, command, status, created_at, stopped_at,
         scratch_disk_size_gb, ram_bytes, total_requests, allowed_requests, denied_requests,
         total_input_tokens, total_output_tokens, total_estimated_cost,
         total_tool_calls, total_mcp_calls, total_file_events,
         compressed_size_bytes, vacuumed_at,
         storage_mode, rootfs_hash, rootfs_version";

    /// Parse a row into a SessionRecord. Column order must match SESSION_COLUMNS.
    fn read_session_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionRecord> {
        Ok(SessionRecord {
            id: row.get(0)?,
            mode: row.get(1)?,
            command: row.get(2)?,
            status: row.get(3)?,
            created_at: row.get(4)?,
            stopped_at: row.get(5)?,
            scratch_disk_size_gb: row.get::<_, i64>(6)? as u32,
            ram_bytes: row.get::<_, i64>(7)? as u64,
            total_requests: row.get::<_, i64>(8)? as u64,
            allowed_requests: row.get::<_, i64>(9)? as u64,
            denied_requests: row.get::<_, i64>(10)? as u64,
            total_input_tokens: row.get::<_, i64>(11)? as u64,
            total_output_tokens: row.get::<_, i64>(12)? as u64,
            total_estimated_cost: row.get::<_, f64>(13)?,
            total_tool_calls: row.get::<_, i64>(14)? as u64,
            total_mcp_calls: row.get::<_, i64>(15)? as u64,
            total_file_events: row.get::<_, i64>(16)? as u64,
            compressed_size_bytes: row.get::<_, Option<i64>>(17)?.map(|v| v as u64),
            vacuumed_at: row.get(18)?,
            storage_mode: row.get::<_, Option<String>>(19)?.unwrap_or_else(|| "block".to_string()),
            rootfs_hash: row.get(20)?,
            rootfs_version: row.get(21)?,
        })
    }

    /// Query the most recent N sessions, newest first.
    pub fn recent(&self, limit: usize) -> rusqlite::Result<Vec<SessionRecord>> {
        let sql = format!(
            "SELECT {} FROM sessions ORDER BY created_at DESC LIMIT ?1",
            Self::SESSION_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![limit as i64], Self::read_session_row)?;
        rows.collect()
    }

    /// Terminate sessions with created_at older than `days` days ago.
    /// Sets status='terminated' on stopped/crashed/vacuumed sessions (not running).
    /// Returns count of affected rows.
    pub fn terminate_older_than_days(&self, days: u32) -> rusqlite::Result<usize> {
        let cutoff_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .saturating_sub(days as u64 * 86400);
        // created_at is ISO 8601 -- string comparison works for our format.
        let cutoff_str = epoch_to_iso(cutoff_secs);
        let count = self.conn.execute(
            "UPDATE sessions SET status = 'terminated'
             WHERE created_at < ?1 AND status IN ('stopped', 'crashed', 'vacuumed')",
            params![cutoff_str],
        )?;
        Ok(count)
    }

    /// Terminate oldest sessions beyond the cap.
    /// Sets status='terminated' on excess stopped/crashed/vacuumed sessions.
    /// Returns count of affected rows.
    pub fn terminate_excess_sessions(&self, max: usize) -> rusqlite::Result<usize> {
        let count = self.conn.execute(
            "UPDATE sessions SET status = 'terminated'
             WHERE status IN ('stopped', 'crashed', 'vacuumed')
             AND id NOT IN (
                SELECT id FROM sessions ORDER BY created_at DESC LIMIT ?1
             )",
            params![max as i64],
        )?;
        Ok(count)
    }

    /// Return stopped/crashed/vacuumed sessions ordered oldest first (for disk culling).
    pub fn stopped_sessions_oldest_first(&self) -> rusqlite::Result<Vec<SessionRecord>> {
        let sql = format!(
            "SELECT {} FROM sessions WHERE status IN ('stopped', 'crashed', 'vacuumed') ORDER BY created_at ASC",
            Self::SESSION_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], Self::read_session_row)?;
        rows.collect()
    }

    /// Return sessions with a specific status.
    pub fn sessions_by_status(&self, status: &str) -> rusqlite::Result<Vec<SessionRecord>> {
        let sql = format!(
            "SELECT {} FROM sessions WHERE status = ?1 ORDER BY created_at ASC",
            Self::SESSION_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![status], Self::read_session_row)?;
        rows.collect()
    }

    /// Return stopped/crashed sessions that have not been vacuumed yet.
    pub fn unvacuumed_sessions(&self) -> rusqlite::Result<Vec<SessionRecord>> {
        let sql = format!(
            "SELECT {} FROM sessions WHERE status IN ('stopped', 'crashed') AND vacuumed_at IS NULL ORDER BY created_at ASC",
            Self::SESSION_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], Self::read_session_row)?;
        rows.collect()
    }

    /// Mark a session as vacuumed with compressed size and timestamp.
    pub fn mark_vacuumed(
        &self,
        id: &str,
        compressed_size_bytes: u64,
        vacuumed_at: &str,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE sessions SET status = 'vacuumed', compressed_size_bytes = ?1, vacuumed_at = ?2 WHERE id = ?3",
            params![compressed_size_bytes as i64, vacuumed_at, id],
        )?;
        Ok(())
    }

    /// Mark a session as terminated (disk artifacts deleted, record retained).
    pub fn mark_terminated(&self, id: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE sessions SET status = 'terminated' WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    /// Checkpoint the main.db WAL (flush and truncate).
    pub fn checkpoint(&self) -> rusqlite::Result<()> {
        self.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
        Ok(())
    }

    /// Permanently delete terminated session records older than `days` days.
    pub fn purge_terminated_older_than_days(&self, days: u32) -> rusqlite::Result<usize> {
        let cutoff_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .saturating_sub(days as u64 * 86400);
        let cutoff_str = epoch_to_iso(cutoff_secs);
        let count = self.conn.execute(
            "DELETE FROM sessions WHERE status = 'terminated' AND created_at < ?1",
            params![cutoff_str],
        )?;
        Ok(count)
    }

    /// Total count of sessions.
    pub fn count(&self) -> rusqlite::Result<usize> {
        self.conn.query_row(
            "SELECT COUNT(*) FROM sessions",
            [],
            |row| row.get::<_, i64>(0).map(|n| n as usize),
        )
    }

    // -- Cross-session aggregation reads ------------------------------------

    /// Global stats aggregated across all sessions.
    pub fn global_stats(&self) -> rusqlite::Result<GlobalStats> {
        self.conn.query_row(
            "SELECT
                COUNT(*),
                COALESCE(SUM(total_input_tokens), 0),
                COALESCE(SUM(total_output_tokens), 0),
                COALESCE(SUM(total_estimated_cost), 0.0),
                COALESCE(SUM(total_tool_calls), 0),
                COALESCE(SUM(total_mcp_calls), 0),
                COALESCE(SUM(total_file_events), 0),
                COALESCE(SUM(total_requests), 0),
                COALESCE(SUM(allowed_requests), 0),
                COALESCE(SUM(denied_requests), 0)
             FROM sessions",
            [],
            |row| {
                Ok(GlobalStats {
                    total_sessions: row.get::<_, i64>(0)? as u64,
                    total_input_tokens: row.get::<_, i64>(1)? as u64,
                    total_output_tokens: row.get::<_, i64>(2)? as u64,
                    total_estimated_cost: row.get::<_, f64>(3)?,
                    total_tool_calls: row.get::<_, i64>(4)? as u64,
                    total_mcp_calls: row.get::<_, i64>(5)? as u64,
                    total_file_events: row.get::<_, i64>(6)? as u64,
                    total_requests: row.get::<_, i64>(7)? as u64,
                    total_allowed: row.get::<_, i64>(8)? as u64,
                    total_denied: row.get::<_, i64>(9)? as u64,
                })
            },
        )
    }

    /// Top providers by call count across all sessions.
    pub fn top_providers(&self, limit: usize) -> rusqlite::Result<Vec<ProviderSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT provider,
                    SUM(call_count),
                    SUM(input_tokens),
                    SUM(output_tokens),
                    SUM(estimated_cost),
                    SUM(total_duration_ms)
             FROM ai_usage
             GROUP BY provider
             ORDER BY SUM(call_count) DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(ProviderSummary {
                provider: row.get(0)?,
                call_count: row.get::<_, i64>(1)? as u64,
                input_tokens: row.get::<_, i64>(2)? as u64,
                output_tokens: row.get::<_, i64>(3)? as u64,
                estimated_cost: row.get::<_, f64>(4)?,
                total_duration_ms: row.get::<_, i64>(5)? as u64,
            })
        })?;
        rows.collect()
    }

    /// Top tools by call count across all sessions.
    pub fn top_tools(&self, limit: usize) -> rusqlite::Result<Vec<ToolSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT tool_name,
                    SUM(call_count),
                    SUM(total_bytes),
                    SUM(total_duration_ms)
             FROM tool_usage
             GROUP BY tool_name
             ORDER BY SUM(call_count) DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(ToolSummary {
                tool_name: row.get(0)?,
                call_count: row.get::<_, i64>(1)? as u64,
                total_bytes: row.get::<_, i64>(2)? as u64,
                total_duration_ms: row.get::<_, i64>(3)? as u64,
            })
        })?;
        rows.collect()
    }

    /// Top MCP tools by call count across all sessions.
    pub fn top_mcp_tools(&self, limit: usize) -> rusqlite::Result<Vec<McpToolSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT tool_name,
                    server_name,
                    SUM(call_count),
                    SUM(total_bytes),
                    SUM(total_duration_ms)
             FROM mcp_usage
             GROUP BY tool_name
             ORDER BY SUM(call_count) DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(McpToolSummary {
                tool_name: row.get(0)?,
                server_name: row.get(1)?,
                call_count: row.get::<_, i64>(2)? as u64,
                total_bytes: row.get::<_, i64>(3)? as u64,
                total_duration_ms: row.get::<_, i64>(4)? as u64,
            })
        })?;
        rows.collect()
    }

    // -- Raw SQL query ------------------------------------------------------

    /// Execute an arbitrary read-only SQL query with optional bind parameters
    /// against main.db. Returns columnar JSON: `{"columns":[...],"rows":[[...], ...]}`.
    /// Caps output at 10,000 rows.
    pub fn query_raw(&self, sql: &str, params: &[serde_json::Value]) -> Result<String, String> {
        // Defense-in-depth: this connection is read-write (used by other
        // SessionIndex methods), so temporarily enable query_only to prevent
        // writes even if validate_select_only is bypassed (e.g. semicolon
        // injection like "SELECT 1; DROP TABLE sessions").
        self.conn
            .pragma_update(None, "query_only", "ON")
            .map_err(|e| e.to_string())?;

        let result = self.query_raw_inner(sql, params);

        // Always restore write capability for other methods, even on error.
        let _ = self.conn.pragma_update(None, "query_only", "OFF");

        result
    }

    fn query_raw_inner(&self, sql: &str, params: &[serde_json::Value]) -> Result<String, String> {
        use serde_json::Value;

        const MAX_ROWS: usize = 10_000;

        let mut stmt = self.conn.prepare(sql).map_err(|e| e.to_string())?;
        let columns: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
        let col_count = columns.len();

        // Convert serde_json::Value to rusqlite dynamic params.
        let rusqlite_params: Vec<Box<dyn rusqlite::types::ToSql>> = params.iter().map(|v| {
            let boxed: Box<dyn rusqlite::types::ToSql> = match v {
                Value::Null => Box::new(rusqlite::types::Null),
                Value::Bool(b) => Box::new(*b as i64),
                Value::Number(n) => {
                    if let Some(i) = n.as_i64() {
                        Box::new(i)
                    } else if let Some(f) = n.as_f64() {
                        Box::new(f)
                    } else {
                        Box::new(rusqlite::types::Null)
                    }
                }
                Value::String(s) => Box::new(s.clone()),
                _ => Box::new(rusqlite::types::Null),
            };
            boxed
        }).collect();
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = rusqlite_params.iter().map(|b| b.as_ref()).collect();

        let mut rows: Vec<Vec<Value>> = Vec::new();
        let mut raw_rows = stmt.query(param_refs.as_slice()).map_err(|e| e.to_string())?;

        while let Some(row) = raw_rows.next().map_err(|e| e.to_string())? {
            if rows.len() >= MAX_ROWS {
                break;
            }
            let mut values = Vec::with_capacity(col_count);
            for i in 0..col_count {
                let val = row.get_ref(i).map_err(|e| e.to_string())?;
                let json_val = match val {
                    rusqlite::types::ValueRef::Null => Value::Null,
                    rusqlite::types::ValueRef::Integer(n) => {
                        Value::Number(serde_json::Number::from(n))
                    }
                    rusqlite::types::ValueRef::Real(f) => {
                        if f.is_finite() {
                            serde_json::Number::from_f64(f)
                                .map(Value::Number)
                                .unwrap_or(Value::Null)
                        } else {
                            Value::Null
                        }
                    }
                    rusqlite::types::ValueRef::Text(t) => {
                        let s = std::str::from_utf8(t).unwrap_or("<invalid utf8>");
                        Value::String(s.to_string())
                    }
                    rusqlite::types::ValueRef::Blob(b) => {
                        Value::String(format!("<blob {} bytes>", b.len()))
                    }
                };
                values.push(json_val);
            }
            rows.push(values);
        }

        let result = serde_json::json!({
            "columns": columns,
            "rows": rows,
        });
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    // -- Per-session summary writes -----------------------------------------

    /// Update the summary columns on a session row.
    pub fn update_session_summary(
        &self,
        id: &str,
        input_tokens: u64,
        output_tokens: u64,
        cost: f64,
        tool_calls: u64,
        mcp_calls: u64,
        file_events: u64,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE sessions SET
                total_input_tokens = ?1,
                total_output_tokens = ?2,
                total_estimated_cost = ?3,
                total_tool_calls = ?4,
                total_mcp_calls = ?5,
                total_file_events = ?6
             WHERE id = ?7",
            params![
                input_tokens as i64,
                output_tokens as i64,
                cost,
                tool_calls as i64,
                mcp_calls as i64,
                file_events as i64,
                id,
            ],
        )?;
        Ok(())
    }

    /// Replace all AI usage rows for a session (DELETE + INSERT batch).
    pub fn replace_ai_usage(
        &self,
        session_id: &str,
        usage: &[ProviderSummary],
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "DELETE FROM ai_usage WHERE session_id = ?1",
            params![session_id],
        )?;
        let mut stmt = self.conn.prepare(
            "INSERT INTO ai_usage (session_id, provider, call_count, input_tokens, output_tokens,
                estimated_cost, total_duration_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;
        for u in usage {
            stmt.execute(params![
                session_id,
                u.provider,
                u.call_count as i64,
                u.input_tokens as i64,
                u.output_tokens as i64,
                u.estimated_cost,
                u.total_duration_ms as i64,
            ])?;
        }
        Ok(())
    }

    /// Replace all tool usage rows for a session (DELETE + INSERT batch).
    pub fn replace_tool_usage(
        &self,
        session_id: &str,
        usage: &[ToolSummary],
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "DELETE FROM tool_usage WHERE session_id = ?1",
            params![session_id],
        )?;
        let mut stmt = self.conn.prepare(
            "INSERT INTO tool_usage (session_id, tool_name, call_count, total_bytes, total_duration_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for u in usage {
            stmt.execute(params![
                session_id,
                u.tool_name,
                u.call_count as i64,
                u.total_bytes as i64,
                u.total_duration_ms as i64,
            ])?;
        }
        Ok(())
    }

    /// Replace all MCP tool usage rows for a session (DELETE + INSERT batch).
    pub fn replace_mcp_usage(
        &self,
        session_id: &str,
        usage: &[McpToolSummary],
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "DELETE FROM mcp_usage WHERE session_id = ?1",
            params![session_id],
        )?;
        let mut stmt = self.conn.prepare(
            "INSERT INTO mcp_usage (session_id, tool_name, server_name, call_count, total_bytes, total_duration_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        for u in usage {
            stmt.execute(params![
                session_id,
                u.tool_name,
                u.server_name,
                u.call_count as i64,
                u.total_bytes as i64,
                u.total_duration_ms as i64,
            ])?;
        }
        Ok(())
    }
}
