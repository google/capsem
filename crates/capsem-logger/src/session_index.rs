use std::path::Path;

use rusqlite::{params, Connection, OpenFlags};

use crate::session_types::*;

/// Session index database wrapping `~/.capsem/sessions/main.db`.
pub struct SessionIndex {
    pub(crate) conn: Connection,
}

/// Current schema version for main.db.
pub const SCHEMA_VERSION: u32 = 8;

pub const SESSION_SCHEMA: &str = "
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
        total_file_events INTEGER NOT NULL DEFAULT 0,
        storage_mode TEXT NOT NULL DEFAULT 'block',
        rootfs_hash TEXT,
        rootfs_version TEXT,
        forked_from TEXT,
        persistent BOOLEAN NOT NULL DEFAULT 0,
        exec_count INTEGER NOT NULL DEFAULT 0,
        audit_event_count INTEGER NOT NULL DEFAULT 0
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

/// An overflow marker is not a file event; it says some went unrecorded.
const FILE_EVENT_COUNT: &str = "SELECT COUNT(*) FROM fs_events WHERE action != 'overflow'";

/// One count. A missing table is a schema violation and bubbles up, not a zero.
fn count_rows(conn: &Connection, sql: &str) -> rusqlite::Result<i64> {
    conn.query_row(sql, [], |row| row.get(0))
}

pub fn ensure_session_index_schema(path: &Path) -> rusqlite::Result<()> {
    SessionIndex::open(path).map(|_| ())
}

pub fn record_session_start(path: &Path, record: &SessionRecord) -> rusqlite::Result<()> {
    SessionIndex::open(path)?.create_or_mark_running(record)
}

pub fn record_session_stop(
    path: &Path,
    id: &str,
    status: &str,
    stopped_at: Option<&str>,
    session_db_path: Option<&Path>,
) -> rusqlite::Result<()> {
    let idx = SessionIndex::open(path)?;
    if let Some(session_db_path) = session_db_path {
        idx.update_session_rollup_from_session_db(id, status, stopped_at, session_db_path)
    } else {
        idx.update_status(id, status, stopped_at)
    }
}

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
    ///
    /// Every branch below lands on the current shape, so each one applies
    /// every change introduced after the version it starts from. They used to
    /// stamp `SCHEMA_VERSION` after a single jump, which marked a v3, v4 or v5
    /// ledger current while it was still missing the `exec_count` and
    /// `audit_event_count` columns that v6->v7 adds.
    pub(crate) fn ensure_schema(conn: &Connection) -> rusqlite::Result<()> {
        let version: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version >= SCHEMA_VERSION {
            // Already at current version -- just ensure tables exist.
            conn.execute_batch(SESSION_SCHEMA)?;
            return Ok(());
        }
        if version < 2 {
            // Old schema -- drop and recreate.
            conn.execute_batch(
                "DROP TABLE IF EXISTS sessions;
                 DROP TABLE IF EXISTS ai_usage;
                 DROP TABLE IF EXISTS tool_usage;
                 DROP TABLE IF EXISTS mcp_usage;",
            )?;
            conn.execute_batch(SESSION_SCHEMA)?;
            conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            return Ok(());
        }

        // v2 and v3 predate the VirtioFS storage columns. v2's own upgrade
        // used to add `compressed_size_bytes` and `vacuumed_at` here; the v8
        // step below drops them, so they are not added in the first place.
        if version <= 3 {
            conn.execute_batch(
                "ALTER TABLE sessions ADD COLUMN storage_mode TEXT NOT NULL DEFAULT 'block';
                 ALTER TABLE sessions ADD COLUMN rootfs_hash TEXT;
                 ALTER TABLE sessions ADD COLUMN rootfs_version TEXT;",
            )?;
        }
        // v5 has the column under its old name; v4 and earlier do not have it.
        if version == 5 {
            conn.execute_batch("ALTER TABLE sessions RENAME COLUMN source_image TO forked_from;")?;
        } else if version <= 4 {
            conn.execute_batch(
                "ALTER TABLE sessions ADD COLUMN forked_from TEXT;
                 ALTER TABLE sessions ADD COLUMN persistent BOOLEAN NOT NULL DEFAULT 0;",
            )?;
        }
        if version <= 6 {
            conn.execute_batch(
                "ALTER TABLE sessions ADD COLUMN exec_count INTEGER NOT NULL DEFAULT 0;
                 ALTER TABLE sessions ADD COLUMN audit_event_count INTEGER NOT NULL DEFAULT 0;",
            )?;
        }
        Self::drop_vacuum_lifecycle(conn)?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        Ok(())
    }

    /// v7 -> v8: the vacuum lifecycle is gone.
    ///
    /// `vacuum_and_compress_session_db` checkpointed a session ledger,
    /// VACUUMed it, gzipped it and deleted the original; `mark_vacuumed`
    /// recorded the result here. Nothing ever called it, and it cannot come
    /// back as written: bodies now live in an append-only `session.bodies`
    /// that `event_body_blobs` indexes by block offset, so rewriting or
    /// removing `session.db` orphans the archive beside it.
    ///
    /// `main.db` outlives every build on a developer's machine, so the columns
    /// have to be dropped rather than left to rot. The presence check is a
    /// fact about this file -- a ledger upgrading from v2 never had them --
    /// not tolerance for an unknown shape: anything other than absence fails.
    fn drop_vacuum_lifecycle(conn: &Connection) -> rusqlite::Result<()> {
        for column in ["compressed_size_bytes", "vacuumed_at"] {
            let present: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('sessions') WHERE name = ?1)",
                params![column],
                |row| row.get(0),
            )?;
            if present {
                conn.execute_batch(&format!("ALTER TABLE sessions DROP COLUMN {column};"))?;
            }
        }
        // A session that reached the dead state is a stopped session; that is
        // all the state ever meant.
        conn.execute("UPDATE sessions SET status = 'stopped' WHERE status = 'vacuumed'", [])?;
        Ok(())
    }

    /// Insert a new session record.
    pub fn create_session(&self, record: &SessionRecord) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO sessions (id, mode, command, status, created_at, stopped_at,
                scratch_disk_size_gb, ram_bytes, total_requests, allowed_requests, denied_requests,
                total_input_tokens, total_output_tokens, total_estimated_cost,
                total_tool_calls, total_file_events,
                storage_mode, rootfs_hash, rootfs_version, forked_from, persistent,
                exec_count, audit_event_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23)",
            params![
                record.id,
                record.mode,
                record.command,
                record.status,
                record.created_at,
                record.stopped_at,
                i64::from(record.scratch_disk_size_gb),
                record.ram_bytes as i64,
                record.total_requests as i64,
                record.allowed_requests as i64,
                record.denied_requests as i64,
                record.total_input_tokens as i64,
                record.total_output_tokens as i64,
                record.total_estimated_cost,
                record.total_tool_calls as i64,
                record.total_file_events as i64,
                record.storage_mode,
                record.rootfs_hash,
                record.rootfs_version,
                record.forked_from,
                record.persistent,
                record.exec_count as i64,
                record.audit_event_count as i64,
            ],
        )?;
        Ok(())
    }

    /// Insert a session start row, or mark an existing row running again.
    ///
    /// Provision retries can reuse the same VM UUID after a boot-before-ready
    /// transient. Keep `create_session` strict for callers/tests that need
    /// duplicate IDs to fail, and use this lifecycle-specific helper when the
    /// retry semantics are intentional.
    pub fn create_or_mark_running(&self, record: &SessionRecord) -> rusqlite::Result<()> {
        match self.create_session(record) {
            Ok(()) => Ok(()),
            Err(rusqlite::Error::SqliteFailure(error, _)) if error.code == rusqlite::ErrorCode::ConstraintViolation => {
                self.update_status(&record.id, "running", None)
            }
            Err(error) => Err(error),
        }
    }

    /// Update session status and optionally set stopped_at.
    pub fn update_status(&self, id: &str, status: &str, stopped_at: Option<&str>) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE sessions SET status = ?1, stopped_at = ?2 WHERE id = ?3",
            params![status, stopped_at, id],
        )?;
        Ok(())
    }

    /// Update request counts for a session.
    pub fn update_request_counts(&self, id: &str, total: u64, allowed: u64, denied: u64) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE sessions SET total_requests = ?1, allowed_requests = ?2, denied_requests = ?3
             WHERE id = ?4",
            params![total as i64, allowed as i64, denied as i64, id],
        )?;
        Ok(())
    }

    /// Mark all "running" sessions as "crashed". Returns count of affected rows.
    pub fn mark_running_as_crashed(&self) -> rusqlite::Result<usize> {
        let count = self
            .conn
            .execute("UPDATE sessions SET status = 'crashed' WHERE status = 'running'", [])?;
        Ok(count)
    }

    /// Shared column list for SELECT queries on sessions.
    const SESSION_COLUMNS: &str = "id, mode, command, status, created_at, stopped_at,
         scratch_disk_size_gb, ram_bytes, total_requests, allowed_requests, denied_requests,
         total_input_tokens, total_output_tokens, total_estimated_cost,
         total_tool_calls, total_file_events,
         storage_mode, rootfs_hash, rootfs_version, forked_from, persistent,
         exec_count, audit_event_count";

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
            total_file_events: row.get::<_, i64>(15)? as u64,
            storage_mode: row.get::<_, Option<String>>(16)?.unwrap_or_else(|| "block".to_string()),
            rootfs_hash: row.get(17)?,
            rootfs_version: row.get(18)?,
            forked_from: row.get(19)?,
            persistent: row.get::<_, Option<bool>>(20)?.unwrap_or(false),
            exec_count: row.get::<_, Option<i64>>(21)?.unwrap_or(0) as u64,
            audit_event_count: row.get::<_, Option<i64>>(22)?.unwrap_or(0) as u64,
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

    /// Return stopped/crashed sessions ordered oldest first (for disk culling).
    pub fn stopped_sessions_oldest_first(&self) -> rusqlite::Result<Vec<SessionRecord>> {
        let sql = format!(
            "SELECT {} FROM sessions WHERE status IN ('stopped', 'crashed') ORDER BY created_at ASC",
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

    /// Mark a session as terminated (disk artifacts deleted, record retained).
    pub fn mark_terminated(&self, id: &str) -> rusqlite::Result<()> {
        self.conn
            .execute("UPDATE sessions SET status = 'terminated' WHERE id = ?1", params![id])?;
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
            .saturating_sub(u64::from(days) * 86400);
        let cutoff_str = epoch_to_iso(cutoff_secs);
        let count = self.conn.execute(
            "DELETE FROM sessions WHERE status = 'terminated' AND created_at < ?1",
            params![cutoff_str],
        )?;
        Ok(count)
    }

    /// Total count of sessions.
    pub fn count(&self) -> rusqlite::Result<usize> {
        count_rows(&self.conn, "SELECT COUNT(*) FROM sessions").map(|n| n as usize)
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
                    total_file_events: row.get::<_, i64>(5)? as u64,
                    total_requests: row.get::<_, i64>(6)? as u64,
                    total_allowed: row.get::<_, i64>(7)? as u64,
                    total_denied: row.get::<_, i64>(8)? as u64,
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
             GROUP BY tool_name, server_name
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
        let rusqlite_params: Vec<Box<dyn rusqlite::types::ToSql>> = params
            .iter()
            .map(|v| {
                let boxed: Box<dyn rusqlite::types::ToSql> = match v {
                    Value::Null => Box::new(rusqlite::types::Null),
                    Value::Bool(b) => Box::new(i64::from(*b)),
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
            })
            .collect();
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
                    rusqlite::types::ValueRef::Integer(n) => Value::Number(serde_json::Number::from(n)),
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
                    rusqlite::types::ValueRef::Blob(b) => Value::String(format!("<blob {} bytes>", b.len())),
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
    #[allow(clippy::too_many_arguments)]
    pub fn update_session_summary(
        &self,
        id: &str,
        input_tokens: u64,
        output_tokens: u64,
        cost: f64,
        tool_calls: u64,
        file_events: u64,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE sessions SET
                total_input_tokens = ?1,
                total_output_tokens = ?2,
                total_estimated_cost = ?3,
                total_tool_calls = ?4,
                total_file_events = ?5
             WHERE id = ?6",
            params![
                input_tokens as i64,
                output_tokens as i64,
                cost,
                tool_calls as i64,
                file_events as i64,
                id,
            ],
        )?;
        Ok(())
    }

    /// Update a main.db session row by rolling up durable counts from its
    /// per-session `session.db`.
    ///
    /// This intentionally queries the canonical session ledger tables directly
    /// inside capsem-logger. Missing tables or columns are schema violations
    /// and bubble up as errors instead of being treated as empty ledgers.
    pub fn update_session_rollup_from_session_db(
        &self,
        id: &str,
        status: &str,
        stopped_at: Option<&str>,
        session_db_path: &Path,
    ) -> rusqlite::Result<()> {
        let session_conn = Connection::open_with_flags(session_db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let (total_requests, allowed_requests, denied_requests): (i64, i64, i64) = session_conn.query_row(
            "SELECT
                    COUNT(*),
                    COALESCE(SUM(CASE WHEN decision = 'allowed' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN decision = 'denied' THEN 1 ELSE 0 END), 0)
                 FROM net_events",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        let (input_tokens, output_tokens, estimated_cost): (i64, i64, f64) = session_conn.query_row(
            "SELECT
                    COALESCE(SUM(input_tokens), 0),
                    COALESCE(SUM(output_tokens), 0),
                    COALESCE(SUM(estimated_cost_usd), 0.0)
                 FROM model_calls",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        let total_tool_calls = count_rows(&session_conn, "SELECT COUNT(*) FROM tool_calls")?;
        let total_file_events = count_rows(&session_conn, FILE_EVENT_COUNT)?;
        let exec_count = count_rows(&session_conn, "SELECT COUNT(*) FROM exec_events")?;
        let audit_event_count = count_rows(&session_conn, "SELECT COUNT(*) FROM audit_events")?;

        let updated = self.conn.execute(
            "UPDATE sessions SET
                status = ?1,
                stopped_at = ?2,
                total_requests = ?3,
                allowed_requests = ?4,
                denied_requests = ?5,
                total_input_tokens = ?6,
                total_output_tokens = ?7,
                total_estimated_cost = ?8,
                total_tool_calls = ?9,
                total_file_events = ?10,
                exec_count = ?11,
                audit_event_count = ?12
             WHERE id = ?13",
            params![
                status,
                stopped_at,
                total_requests,
                allowed_requests,
                denied_requests,
                input_tokens,
                output_tokens,
                estimated_cost,
                total_tool_calls,
                total_file_events,
                exec_count,
                audit_event_count,
                id,
            ],
        )?;
        if updated == 0 {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
        Ok(())
    }

    /// Replace all AI usage rows for a session (DELETE + INSERT batch).
    pub fn replace_ai_usage(&self, session_id: &str, usage: &[ProviderSummary]) -> rusqlite::Result<()> {
        self.conn
            .execute("DELETE FROM ai_usage WHERE session_id = ?1", params![session_id])?;
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
    pub fn replace_tool_usage(&self, session_id: &str, usage: &[ToolSummary]) -> rusqlite::Result<()> {
        self.conn
            .execute("DELETE FROM tool_usage WHERE session_id = ?1", params![session_id])?;
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
    pub fn replace_mcp_usage(&self, session_id: &str, usage: &[McpToolSummary]) -> rusqlite::Result<()> {
        self.conn
            .execute("DELETE FROM mcp_usage WHERE session_id = ?1", params![session_id])?;
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

#[cfg(test)]
mod tests;
