/// Session management: unique session IDs, session index DB, and lifecycle.
///
/// Each VM boot creates a new session with a unique ID (YYYYMMDD-HHMMSS-XXXX).
/// The session index (`main.db`) tracks metadata across sessions. Per-session
/// telemetry lives in `<session_dir>/session.db`.
///
/// Session lifecycle:
///   running -> stopped    (graceful shutdown, rollup done)
///   running -> crashed    (ungraceful, backfill on next startup)
///   stopped/crashed -> vacuumed   (DB checkpointed + vacuumed + gzipped)
///   vacuumed -> terminated        (disk artifacts deleted, only main.db record)
use std::io::Write;
use std::path::Path;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

/// Generate a unique session ID: YYYYMMDD-HHMMSS-XXXX (4 random hex chars).
pub fn generate_session_id() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    let (y, m, d, hours, minutes, seconds) = epoch_to_parts(secs);

    // 4 random hex chars from timestamp nanos + XOR with a counter.
    let nanos = now.subsec_nanos();
    let rand_bits = nanos ^ std::process::id().wrapping_mul(2654435761);
    let suffix = rand_bits & 0xFFFF;

    format!(
        "{y:04}{m:02}{d:02}-{hours:02}{minutes:02}{seconds:02}-{suffix:04x}",
    )
}

/// Validate that a string looks like a valid session ID.
pub fn is_valid_session_id(s: &str) -> bool {
    // YYYYMMDD-HHMMSS-XXXX = 20 chars
    if s.len() != 20 {
        return false;
    }
    let bytes = s.as_bytes();
    // Check structure: 8 digits, dash, 6 digits, dash, 4 hex
    bytes[0..8].iter().all(|b| b.is_ascii_digit())
        && bytes[8] == b'-'
        && bytes[9..15].iter().all(|b| b.is_ascii_digit())
        && bytes[15] == b'-'
        && bytes[16..20].iter().all(|b| b.is_ascii_hexdigit())
}

/// A session record stored in main.db.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: String,
    pub mode: String,
    pub command: Option<String>,
    pub status: String,
    pub created_at: String,
    pub stopped_at: Option<String>,
    pub scratch_disk_size_gb: u32,
    pub ram_bytes: u64,
    pub total_requests: u64,
    pub allowed_requests: u64,
    pub denied_requests: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_estimated_cost: f64,
    pub total_tool_calls: u64,
    pub total_mcp_calls: u64,
    pub total_file_events: u64,
    pub compressed_size_bytes: Option<u64>,
    pub vacuumed_at: Option<String>,
}

/// Aggregated statistics across all sessions.
#[derive(Debug, Clone, Serialize)]
pub struct GlobalStats {
    pub total_sessions: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_estimated_cost: f64,
    pub total_tool_calls: u64,
    pub total_mcp_calls: u64,
    pub total_file_events: u64,
    pub total_requests: u64,
    pub total_allowed: u64,
    pub total_denied: u64,
}

/// Per-provider AI usage summary across sessions.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderSummary {
    pub provider: String,
    pub call_count: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub estimated_cost: f64,
    pub total_duration_ms: u64,
}

/// Per-tool usage summary across sessions.
#[derive(Debug, Clone, Serialize)]
pub struct ToolSummary {
    pub tool_name: String,
    pub call_count: u64,
    pub total_bytes: u64,
    pub total_duration_ms: u64,
}

/// Per-MCP-tool usage summary across sessions.
#[derive(Debug, Clone, Serialize)]
pub struct McpToolSummary {
    pub tool_name: String,
    pub server_name: String,
    pub call_count: u64,
    pub total_bytes: u64,
    pub total_duration_ms: u64,
}

/// Session index database wrapping `~/.capsem/sessions/main.db`.
pub struct SessionIndex {
    conn: Connection,
}

/// Current schema version for main.db.
const SCHEMA_VERSION: u32 = 3;

const SESSION_SCHEMA: &str = "
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
        vacuumed_at TEXT
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
    fn ensure_schema(conn: &Connection) -> rusqlite::Result<()> {
        let version: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version == 2 {
            // Additive migration v2->v3: add new nullable columns.
            conn.execute_batch(
                "ALTER TABLE sessions ADD COLUMN compressed_size_bytes INTEGER;
                 ALTER TABLE sessions ADD COLUMN vacuumed_at TEXT;"
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
                compressed_size_bytes, vacuumed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
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
         compressed_size_bytes, vacuumed_at";

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

    // ── Cross-session aggregation reads ──────────────────────────────

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

    // ── Raw SQL query ─────────────────────────────────────────────

    /// Execute an arbitrary read-only SQL query with optional bind parameters
    /// against main.db. Returns columnar JSON: `{"columns":[...],"rows":[[...], ...]}`.
    /// Caps output at 10,000 rows.
    pub fn query_raw(&self, sql: &str, params: &[serde_json::Value]) -> Result<String, String> {
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

    // ── Per-session summary writes ──────────────────────────────────

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

/// Checkpoint, vacuum, and gzip-compress a session database.
///
/// 1. Opens `session.db` in the given directory
/// 2. Checkpoints WAL (TRUNCATE mode)
/// 3. VACUUMs the database
/// 4. Closes the connection
/// 5. Gzip-compresses to `session.db.gz`
/// 6. Removes `session.db`, `session.db-wal`, `session.db-shm`
///
/// Returns the compressed file size in bytes.
pub fn vacuum_and_compress_session_db(session_dir: &Path) -> anyhow::Result<u64> {
    let db_path = session_dir.join("session.db");
    if !db_path.exists() {
        return Err(anyhow::anyhow!("session.db not found in {}", session_dir.display()));
    }

    // Open, checkpoint, vacuum, close.
    {
        let conn = Connection::open(&db_path)?;
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
        conn.execute_batch("VACUUM")?;
    }

    // Gzip compress session.db -> session.db.gz.
    let gz_path = session_dir.join("session.db.gz");
    let input = std::fs::read(&db_path)?;
    {
        let gz_file = std::fs::File::create(&gz_path)?;
        let mut encoder = flate2::write::GzEncoder::new(gz_file, flate2::Compression::default());
        encoder.write_all(&input)?;
        encoder.finish()?;
    }

    let compressed_size = std::fs::metadata(&gz_path)?.len();

    // Remove uncompressed files.
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(session_dir.join("session.db-wal"));
    let _ = std::fs::remove_file(session_dir.join("session.db-shm"));

    Ok(compressed_size)
}

/// Break epoch seconds into (year, month, day, hour, minute, second) UTC components.
fn epoch_to_parts(secs: u64) -> (i64, u32, u32, u64, u64, u64) {
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    let mut y = 1970i64;
    let mut remaining_days = days as i64;
    loop {
        let year_days = if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
            366
        } else {
            365
        };
        if remaining_days < year_days {
            break;
        }
        remaining_days -= year_days;
        y += 1;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let month_days = [
        31,
        if leap { 29 } else { 28 },
        31, 30, 31, 30, 31, 31, 30, 31, 30, 31,
    ];
    let mut m = 0u32;
    for md in &month_days {
        if remaining_days < *md {
            break;
        }
        remaining_days -= md;
        m += 1;
    }
    (y, m + 1, remaining_days as u32 + 1, hours, minutes, seconds)
}

/// Convert epoch seconds to ISO 8601 string (YYYY-MM-DDTHH:MM:SSZ).
pub fn epoch_to_iso(secs: u64) -> String {
    let (y, m, d, hours, minutes, seconds) = epoch_to_parts(secs);
    format!("{y:04}-{m:02}-{d:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

/// Current UTC time as ISO 8601 string.
pub fn now_iso() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    epoch_to_iso(secs)
}

/// Calculate total disk usage in bytes for all session directories under the given base path.
pub fn disk_usage_bytes(sessions_base: &Path) -> u64 {
    let entries = match std::fs::read_dir(sessions_base) {
        Ok(e) => e,
        Err(_) => return 0,
    };
    let mut total = 0u64;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            total += dir_size(&path);
        } else if path.is_file() {
            total += path.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }
    total
}

fn dir_size(path: &Path) -> u64 {
    let entries = match std::fs::read_dir(path) {
        Ok(e) => e,
        Err(_) => return 0,
    };
    let mut total = 0u64;
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            total += dir_size(&p);
        } else if p.is_file() {
            total += p.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

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
            total_mcp_calls: 0,
            total_file_events: 0,
            compressed_size_bytes: None,
            vacuumed_at: None,
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
        assert_eq!(
            records[0].stopped_at.as_deref(),
            Some("2026-02-25T15:00:00Z")
        );
    }

    #[test]
    fn update_status_nonexistent_is_noop() {
        let idx = SessionIndex::open_in_memory().unwrap();
        // Should not crash.
        idx.update_status("nonexistent", "stopped", None).unwrap();
    }

    #[test]
    fn update_request_counts_works() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "running"))
            .unwrap();
        idx.update_request_counts("20260225-143052-a7f3", 10, 7, 3)
            .unwrap();
        let records = idx.recent(1).unwrap();
        assert_eq!(records[0].total_requests, 10);
        assert_eq!(records[0].allowed_requests, 7);
        assert_eq!(records[0].denied_requests, 3);
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

    // -- Age-based culling --

    #[test]
    fn terminate_older_than_days() {
        let idx = SessionIndex::open_in_memory().unwrap();

        // Old session (2020).
        let mut old = sample_record("20200101-120000-0000", "stopped");
        old.created_at = "2020-01-01T12:00:00Z".to_string();
        idx.create_session(&old).unwrap();

        // Recent session.
        let mut recent = sample_record("20260225-143052-a7f3", "stopped");
        recent.created_at = "2026-02-25T14:30:52Z".to_string();
        idx.create_session(&recent).unwrap();

        let terminated = idx.terminate_older_than_days(7).unwrap();
        assert_eq!(terminated, 1);
        // Row still exists, just status changed.
        assert_eq!(idx.count().unwrap(), 2);
        let records = idx.recent(10).unwrap();
        let old_rec = records.iter().find(|r| r.id == "20200101-120000-0000").unwrap();
        assert_eq!(old_rec.status, "terminated");
    }

    #[test]
    fn terminate_older_preserves_running() {
        let idx = SessionIndex::open_in_memory().unwrap();

        let mut old_running = sample_record("20200101-120000-0000", "running");
        old_running.created_at = "2020-01-01T12:00:00Z".to_string();
        idx.create_session(&old_running).unwrap();

        let terminated = idx.terminate_older_than_days(7).unwrap();
        assert_eq!(terminated, 0);
        assert_eq!(idx.recent(1).unwrap()[0].status, "running");
    }

    #[test]
    fn terminate_older_includes_vacuumed() {
        let idx = SessionIndex::open_in_memory().unwrap();

        let mut old = sample_record("20200101-120000-0000", "vacuumed");
        old.created_at = "2020-01-01T12:00:00Z".to_string();
        idx.create_session(&old).unwrap();

        let terminated = idx.terminate_older_than_days(7).unwrap();
        assert_eq!(terminated, 1);
        let records = idx.recent(1).unwrap();
        assert_eq!(records[0].status, "terminated");
    }

    // -- Count-based culling --

    #[test]
    fn terminate_excess_sessions() {
        let idx = SessionIndex::open_in_memory().unwrap();
        for i in 0..5 {
            let mut rec = sample_record(&format!("20260225-{i:06}-0000"), "stopped");
            rec.created_at = format!("2026-02-25T{i:02}:00:00Z");
            idx.create_session(&rec).unwrap();
        }
        let terminated = idx.terminate_excess_sessions(3).unwrap();
        assert_eq!(terminated, 2);
        // All rows still exist, 2 are now terminated.
        assert_eq!(idx.count().unwrap(), 5);
        let terminated_recs = idx.sessions_by_status("terminated").unwrap();
        assert_eq!(terminated_recs.len(), 2);
    }

    #[test]
    fn terminate_excess_ignores_running() {
        let idx = SessionIndex::open_in_memory().unwrap();
        for i in 0..3 {
            let mut rec = sample_record(&format!("20260225-{i:06}-0000"), "stopped");
            rec.created_at = format!("2026-02-25T{i:02}:00:00Z");
            idx.create_session(&rec).unwrap();
        }
        let mut running = sample_record("20260225-100000-0000", "running");
        running.created_at = "2026-02-24T00:00:00Z".to_string();
        idx.create_session(&running).unwrap();

        let terminated = idx.terminate_excess_sessions(2).unwrap();
        assert_eq!(terminated, 1);
        // running session untouched.
        let r = idx.recent(10).unwrap();
        assert!(r.iter().any(|rec| rec.id == "20260225-100000-0000" && rec.status == "running"));
    }

    #[test]
    fn terminate_excess_noop_under_cap() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "stopped"))
            .unwrap();
        let terminated = idx.terminate_excess_sessions(10).unwrap();
        assert_eq!(terminated, 0);
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

    // -- Disk usage --

    #[test]
    fn disk_usage_bytes_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(disk_usage_bytes(dir.path()), 0);
    }

    #[test]
    fn disk_usage_bytes_with_files() {
        let dir = tempfile::tempdir().unwrap();
        let session = dir.path().join("20260225-143052-a7f3");
        std::fs::create_dir_all(&session).unwrap();
        std::fs::write(session.join("info.db"), vec![0u8; 4096]).unwrap();
        let usage = disk_usage_bytes(dir.path());
        assert!(usage >= 4096, "usage={usage}");
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
        let version: u32 = idx.conn.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn schema_upgrade_from_v0() {
        // Simulate a v0 DB (no user_version set = 0).
        let conn = Connection::open_in_memory().unwrap();
        // Create old-style sessions table without new columns.
        conn.execute_batch("CREATE TABLE sessions (id TEXT PRIMARY KEY, mode TEXT NOT NULL)").unwrap();
        conn.execute("INSERT INTO sessions (id, mode) VALUES ('old', 'gui')", []).unwrap();
        // Now ensure_schema should drop and recreate (v0 < v2 path).
        SessionIndex::ensure_schema(&conn).unwrap();
        let version: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        // Old data is gone (clean slate).
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn schema_upgrade_from_v1() {
        // v1 < v2, so same drop+recreate behavior.
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", 1u32).unwrap();
        conn.execute_batch("CREATE TABLE sessions (id TEXT PRIMARY KEY)").unwrap();
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
            idx.create_session(&sample_record("20260225-143052-a7f3", "running")).unwrap();
        }
        // Reopen -- same version, data preserved.
        let idx = SessionIndex::open(&path).unwrap();
        assert_eq!(idx.count().unwrap(), 1);
    }

    // -- New columns default to zero --

    #[test]
    fn new_columns_default_to_zero() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "running")).unwrap();
        let records = idx.recent(1).unwrap();
        assert_eq!(records[0].total_input_tokens, 0);
        assert_eq!(records[0].total_output_tokens, 0);
        assert_eq!(records[0].total_estimated_cost, 0.0);
        assert_eq!(records[0].total_tool_calls, 0);
        assert_eq!(records[0].total_mcp_calls, 0);
        assert_eq!(records[0].total_file_events, 0);
    }

    // -- update_session_summary --

    #[test]
    fn update_session_summary_works() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "running")).unwrap();
        idx.update_session_summary("20260225-143052-a7f3", 1000, 500, 0.15, 42, 5, 100).unwrap();
        let records = idx.recent(1).unwrap();
        assert_eq!(records[0].total_input_tokens, 1000);
        assert_eq!(records[0].total_output_tokens, 500);
        assert!((records[0].total_estimated_cost - 0.15).abs() < 1e-6);
        assert_eq!(records[0].total_tool_calls, 42);
        assert_eq!(records[0].total_mcp_calls, 5);
        assert_eq!(records[0].total_file_events, 100);
    }

    // -- global_stats --

    #[test]
    fn global_stats_empty() {
        let idx = SessionIndex::open_in_memory().unwrap();
        let gs = idx.global_stats().unwrap();
        assert_eq!(gs.total_sessions, 0);
        assert_eq!(gs.total_input_tokens, 0);
        assert_eq!(gs.total_estimated_cost, 0.0);
    }

    #[test]
    fn global_stats_multi_session() {
        let idx = SessionIndex::open_in_memory().unwrap();
        let mut r1 = sample_record("20260225-143052-a7f3", "stopped");
        r1.total_input_tokens = 1000;
        r1.total_output_tokens = 500;
        r1.total_estimated_cost = 0.10;
        r1.total_tool_calls = 20;
        r1.total_mcp_calls = 3;
        r1.total_file_events = 50;
        r1.total_requests = 10;
        r1.allowed_requests = 8;
        r1.denied_requests = 2;
        idx.create_session(&r1).unwrap();

        let mut r2 = sample_record("20260225-143053-b8e4", "stopped");
        r2.created_at = "2026-02-25T14:30:53Z".to_string();
        r2.total_input_tokens = 2000;
        r2.total_output_tokens = 1000;
        r2.total_estimated_cost = 0.20;
        r2.total_tool_calls = 30;
        r2.total_mcp_calls = 7;
        r2.total_file_events = 25;
        r2.total_requests = 5;
        r2.allowed_requests = 4;
        r2.denied_requests = 1;
        idx.create_session(&r2).unwrap();

        let gs = idx.global_stats().unwrap();
        assert_eq!(gs.total_sessions, 2);
        assert_eq!(gs.total_input_tokens, 3000);
        assert_eq!(gs.total_output_tokens, 1500);
        assert!((gs.total_estimated_cost - 0.30).abs() < 1e-6);
        assert_eq!(gs.total_tool_calls, 50);
        assert_eq!(gs.total_mcp_calls, 10);
        assert_eq!(gs.total_file_events, 75);
        assert_eq!(gs.total_requests, 15);
        assert_eq!(gs.total_allowed, 12);
        assert_eq!(gs.total_denied, 3);
    }

    // -- replace_ai_usage + top_providers --

    #[test]
    fn replace_ai_usage_and_top_providers() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "stopped")).unwrap();

        let usage = vec![
            ProviderSummary { provider: "anthropic".into(), call_count: 10, input_tokens: 5000, output_tokens: 2000, estimated_cost: 0.10, total_duration_ms: 3000 },
            ProviderSummary { provider: "google".into(), call_count: 5, input_tokens: 2000, output_tokens: 1000, estimated_cost: 0.05, total_duration_ms: 1500 },
        ];
        idx.replace_ai_usage("20260225-143052-a7f3", &usage).unwrap();

        let providers = idx.top_providers(10).unwrap();
        assert_eq!(providers.len(), 2);
        assert_eq!(providers[0].provider, "anthropic"); // highest call_count first
        assert_eq!(providers[0].call_count, 10);
        assert_eq!(providers[1].provider, "google");
    }

    #[test]
    fn replace_ai_usage_replaces_old_data() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "stopped")).unwrap();

        let old = vec![
            ProviderSummary { provider: "anthropic".into(), call_count: 10, input_tokens: 5000, output_tokens: 2000, estimated_cost: 0.10, total_duration_ms: 3000 },
        ];
        idx.replace_ai_usage("20260225-143052-a7f3", &old).unwrap();

        let new = vec![
            ProviderSummary { provider: "openai".into(), call_count: 20, input_tokens: 8000, output_tokens: 4000, estimated_cost: 0.30, total_duration_ms: 5000 },
        ];
        idx.replace_ai_usage("20260225-143052-a7f3", &new).unwrap();

        let providers = idx.top_providers(10).unwrap();
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].provider, "openai");
        assert_eq!(providers[0].call_count, 20);
    }

    // -- replace_tool_usage + top_tools --

    #[test]
    fn replace_tool_usage_and_top_tools() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "stopped")).unwrap();

        let usage = vec![
            ToolSummary { tool_name: "read_file".into(), call_count: 50, total_bytes: 100_000, total_duration_ms: 2000 },
            ToolSummary { tool_name: "write_file".into(), call_count: 30, total_bytes: 50_000, total_duration_ms: 1500 },
        ];
        idx.replace_tool_usage("20260225-143052-a7f3", &usage).unwrap();

        let tools = idx.top_tools(10).unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].tool_name, "read_file"); // highest count first
        assert_eq!(tools[0].call_count, 50);
    }

    // -- replace_mcp_usage + top_mcp_tools --

    #[test]
    fn replace_mcp_usage_and_top_mcp_tools() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "stopped")).unwrap();

        let usage = vec![
            McpToolSummary { tool_name: "github__search".into(), server_name: "github".into(), call_count: 15, total_bytes: 30_000, total_duration_ms: 4500 },
            McpToolSummary { tool_name: "fs__read".into(), server_name: "filesystem".into(), call_count: 8, total_bytes: 10_000, total_duration_ms: 800 },
        ];
        idx.replace_mcp_usage("20260225-143052-a7f3", &usage).unwrap();

        let tools = idx.top_mcp_tools(10).unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].tool_name, "github__search");
        assert_eq!(tools[0].server_name, "github");
        assert_eq!(tools[0].call_count, 15);
    }

    // -- Cross-session aggregation --

    #[test]
    fn top_providers_aggregates_across_sessions() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "stopped")).unwrap();
        let mut r2 = sample_record("20260225-143053-b8e4", "stopped");
        r2.created_at = "2026-02-25T14:30:53Z".to_string();
        idx.create_session(&r2).unwrap();

        idx.replace_ai_usage("20260225-143052-a7f3", &[
            ProviderSummary { provider: "anthropic".into(), call_count: 10, input_tokens: 5000, output_tokens: 2000, estimated_cost: 0.10, total_duration_ms: 3000 },
        ]).unwrap();
        idx.replace_ai_usage("20260225-143053-b8e4", &[
            ProviderSummary { provider: "anthropic".into(), call_count: 5, input_tokens: 2000, output_tokens: 1000, estimated_cost: 0.05, total_duration_ms: 1000 },
        ]).unwrap();

        let providers = idx.top_providers(10).unwrap();
        assert_eq!(providers.len(), 1); // grouped by provider
        assert_eq!(providers[0].call_count, 15);
        assert_eq!(providers[0].input_tokens, 7000);
    }

    // -- Schema migration v2->v3 --

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
                total_mcp_calls INTEGER NOT NULL DEFAULT 0,
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
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 1);

        // New columns exist with NULL defaults.
        let compressed: Option<i64> = conn.query_row(
            "SELECT compressed_size_bytes FROM sessions WHERE id = 'test-id'", [], |row| row.get(0)
        ).unwrap();
        assert!(compressed.is_none());

        let vacuumed: Option<String> = conn.query_row(
            "SELECT vacuumed_at FROM sessions WHERE id = 'test-id'", [], |row| row.get(0)
        ).unwrap();
        assert!(vacuumed.is_none());
    }

    // -- New lifecycle methods --

    #[test]
    fn mark_vacuumed_sets_fields() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "stopped")).unwrap();
        idx.mark_vacuumed("20260225-143052-a7f3", 12345, "2026-02-25T15:00:00Z").unwrap();

        let records = idx.recent(1).unwrap();
        assert_eq!(records[0].status, "vacuumed");
        assert_eq!(records[0].compressed_size_bytes, Some(12345));
        assert_eq!(records[0].vacuumed_at.as_deref(), Some("2026-02-25T15:00:00Z"));
    }

    #[test]
    fn mark_terminated_sets_status() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "vacuumed")).unwrap();
        idx.mark_terminated("20260225-143052-a7f3").unwrap();

        let records = idx.recent(1).unwrap();
        assert_eq!(records[0].status, "terminated");
    }

    #[test]
    fn unvacuumed_sessions_returns_correct_set() {
        let idx = SessionIndex::open_in_memory().unwrap();

        // Stopped without vacuum -- should be returned.
        let mut s1 = sample_record("20260225-100000-0000", "stopped");
        s1.created_at = "2026-02-25T10:00:00Z".to_string();
        idx.create_session(&s1).unwrap();

        // Crashed without vacuum -- should be returned.
        let mut s2 = sample_record("20260225-110000-0000", "crashed");
        s2.created_at = "2026-02-25T11:00:00Z".to_string();
        idx.create_session(&s2).unwrap();

        // Running -- should NOT be returned.
        let mut s3 = sample_record("20260225-120000-0000", "running");
        s3.created_at = "2026-02-25T12:00:00Z".to_string();
        idx.create_session(&s3).unwrap();

        // Already vacuumed -- should NOT be returned.
        let mut s4 = sample_record("20260225-130000-0000", "vacuumed");
        s4.created_at = "2026-02-25T13:00:00Z".to_string();
        s4.vacuumed_at = Some("2026-02-25T14:00:00Z".to_string());
        idx.create_session(&s4).unwrap();

        let unvacuumed = idx.unvacuumed_sessions().unwrap();
        assert_eq!(unvacuumed.len(), 2);
        assert_eq!(unvacuumed[0].id, "20260225-100000-0000");
        assert_eq!(unvacuumed[1].id, "20260225-110000-0000");
    }

    #[test]
    fn sessions_by_status_filters_correctly() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "stopped")).unwrap();
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

        // Recent terminated session.
        let mut recent = sample_record("20260225-143052-a7f3", "terminated");
        recent.created_at = "2026-02-25T14:30:52Z".to_string();
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
        idx.create_session(&sample_record("20260225-143052-a7f3", "running")).unwrap();

        // running -> stopped
        idx.update_status("20260225-143052-a7f3", "stopped", Some("2026-02-25T15:00:00Z")).unwrap();
        assert_eq!(idx.recent(1).unwrap()[0].status, "stopped");

        // stopped -> vacuumed
        idx.mark_vacuumed("20260225-143052-a7f3", 5000, "2026-02-25T15:01:00Z").unwrap();
        let rec = &idx.recent(1).unwrap()[0];
        assert_eq!(rec.status, "vacuumed");
        assert_eq!(rec.compressed_size_bytes, Some(5000));

        // vacuumed -> terminated
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
    fn new_columns_null_by_default() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "running")).unwrap();
        let records = idx.recent(1).unwrap();
        assert!(records[0].compressed_size_bytes.is_none());
        assert!(records[0].vacuumed_at.is_none());
    }

    // -- Vacuum + compress --

    #[test]
    fn vacuum_and_compress_creates_gz_and_removes_db() {
        let dir = tempfile::tempdir().unwrap();
        let session_dir = dir.path().join("20260225-143052-a7f3");
        std::fs::create_dir_all(&session_dir).unwrap();

        // Create a real session DB with some data.
        let db_path = session_dir.join("session.db");
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.pragma_update(None, "journal_mode", "WAL").unwrap();
            conn.execute_batch("CREATE TABLE test (id INTEGER, data TEXT)").unwrap();
            for i in 0..100 {
                conn.execute("INSERT INTO test (id, data) VALUES (?1, ?2)", params![i, format!("row-{i}")]).unwrap();
            }
        }
        // Create fake WAL/SHM files.
        std::fs::write(session_dir.join("session.db-wal"), b"fake wal").unwrap();
        std::fs::write(session_dir.join("session.db-shm"), b"fake shm").unwrap();

        let compressed_size = vacuum_and_compress_session_db(&session_dir).unwrap();
        assert!(compressed_size > 0);

        // .gz exists, .db/.wal/.shm are gone.
        assert!(session_dir.join("session.db.gz").exists());
        assert!(!session_dir.join("session.db").exists());
        assert!(!session_dir.join("session.db-wal").exists());
        assert!(!session_dir.join("session.db-shm").exists());

        // Decompress and verify data integrity.
        let gz_data = std::fs::read(session_dir.join("session.db.gz")).unwrap();
        let mut decoder = flate2::read::GzDecoder::new(&gz_data[..]);
        let mut decompressed = Vec::new();
        std::io::Read::read_to_end(&mut decoder, &mut decompressed).unwrap();

        let temp_db = session_dir.join("verify.db");
        std::fs::write(&temp_db, &decompressed).unwrap();
        let conn = Connection::open(&temp_db).unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM test", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 100);
    }

    #[test]
    fn vacuum_and_compress_nonexistent_db_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let session_dir = dir.path().join("nonexistent");
        std::fs::create_dir_all(&session_dir).unwrap();
        let result = vacuum_and_compress_session_db(&session_dir);
        assert!(result.is_err());
    }

    #[test]
    fn vacuum_and_compress_double_call_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let session_dir = dir.path().join("20260225-143052-a7f3");
        std::fs::create_dir_all(&session_dir).unwrap();

        let db_path = session_dir.join("session.db");
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch("CREATE TABLE test (id INTEGER)").unwrap();
        }

        // First call succeeds.
        vacuum_and_compress_session_db(&session_dir).unwrap();
        assert!(session_dir.join("session.db.gz").exists());

        // Second call fails (no .db file).
        let result = vacuum_and_compress_session_db(&session_dir);
        assert!(result.is_err());
    }

    #[test]
    fn stopped_sessions_includes_vacuumed() {
        let idx = SessionIndex::open_in_memory().unwrap();

        let mut s1 = sample_record("20260225-100000-0000", "stopped");
        s1.created_at = "2026-02-25T10:00:00Z".to_string();
        idx.create_session(&s1).unwrap();

        let mut s2 = sample_record("20260225-110000-0000", "vacuumed");
        s2.created_at = "2026-02-25T11:00:00Z".to_string();
        idx.create_session(&s2).unwrap();

        let mut s3 = sample_record("20260225-120000-0000", "terminated");
        s3.created_at = "2026-02-25T12:00:00Z".to_string();
        idx.create_session(&s3).unwrap();

        let stopped = idx.stopped_sessions_oldest_first().unwrap();
        assert_eq!(stopped.len(), 2); // stopped + vacuumed, not terminated
        assert_eq!(stopped[0].id, "20260225-100000-0000");
        assert_eq!(stopped[1].id, "20260225-110000-0000");
    }

    // -- query_raw --

    #[test]
    fn query_raw_returns_columnar_json() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "running")).unwrap();

        let json_str = idx.query_raw("SELECT id, mode, status FROM sessions", &[]).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["columns"], serde_json::json!(["id", "mode", "status"]));
        assert_eq!(parsed["rows"].as_array().unwrap().len(), 1);
        assert_eq!(parsed["rows"][0][0], "20260225-143052-a7f3");
    }

    #[test]
    fn query_raw_with_bind_params() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "running")).unwrap();
        let mut r2 = sample_record("20260225-143053-b8e4", "stopped");
        r2.created_at = "2026-02-25T14:30:53Z".to_string();
        idx.create_session(&r2).unwrap();

        let params = vec![serde_json::json!("stopped")];
        let json_str = idx.query_raw("SELECT id FROM sessions WHERE status = ?", &params).unwrap();
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
}
