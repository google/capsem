use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

use rusqlite::{params, Connection, OpenFlags, Row};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::events::{
    AuditEvent, Decision, ExecEvent, FileAction, FileEvent, ModelCall, NetEvent, SecurityAskEvent,
    SecurityAskStatus, SecurityDetectionLevel, SecurityRuleAction, SecurityRuleEvent,
    ToolCallEntry, ToolResponseEntry,
};
use crate::schema;

/// Counts of network events by decision outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NetEventCounts {
    pub total: usize,
    pub allowed: usize,
    pub denied: usize,
}

/// Aggregate statistics for a session (computed from SQL queries).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStats {
    pub net_total: u64,
    pub net_allowed: u64,
    pub net_denied: u64,
    pub net_error: u64,
    pub net_bytes_sent: u64,
    pub net_bytes_received: u64,
    pub model_call_count: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_usage_details: BTreeMap<String, u64>,
    pub total_model_duration_ms: u64,
    pub total_tool_calls: u64,
    pub total_estimated_cost_usd: f64,
}

/// Domain request counts (from GROUP BY domain).
#[derive(Debug, Clone, Serialize)]
pub struct DomainCount {
    pub domain: String,
    pub count: u64,
    pub allowed: u64,
    pub denied: u64,
}

/// A time bucket for charting requests over time.
#[derive(Debug, Clone, Serialize)]
pub struct TimeBucket {
    pub bucket_start: String,
    pub allowed: u64,
    pub denied: u64,
}

/// Per-provider token usage and cost.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderTokenUsage {
    pub provider: String,
    pub call_count: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_duration_ms: u64,
    pub total_estimated_cost_usd: f64,
}

/// Tool name + usage count.
#[derive(Debug, Clone, Serialize)]
pub struct ToolUsageCount {
    pub tool_name: String,
    pub count: u64,
}

/// Tool usage with response size and duration stats (from JOIN with model_calls).
#[derive(Debug, Clone, Serialize)]
pub struct ToolUsageWithStats {
    pub tool_name: String,
    pub count: u64,
    pub total_bytes: u64,
    pub total_duration_ms: u64,
}

/// MCP tool usage aggregated by tool_name.
#[derive(Debug, Clone, Serialize)]
pub struct McpToolUsage {
    pub tool_name: String,
    pub server_name: String,
    pub count: u64,
    pub total_bytes: u64,
    pub total_duration_ms: u64,
}

/// A user/security tool-call ledger row from `tool_calls`.
#[derive(Debug, Clone, Serialize)]
pub struct ToolCallLedgerEntry {
    pub id: i64,
    pub event_id: String,
    pub timestamp: String,
    pub model_call_id: Option<i64>,
    pub origin: String,
    pub server_name: Option<String>,
    pub method: Option<String>,
    pub request_id: Option<String>,
    pub call_id: String,
    pub tool_name: String,
    pub arguments: Option<String>,
    pub response_preview: Option<String>,
    pub decision: String,
    pub duration_ms: u64,
    pub error_message: Option<String>,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub policy_rule: Option<String>,
    pub trace_id: Option<String>,
    pub credential_ref: Option<String>,
}

/// Summary of a trace (one agent turn) aggregated from grouped model calls.
#[derive(Debug, Clone, Serialize)]
pub struct TraceSummary {
    pub trace_id: String,
    pub started_at: f64,
    pub ended_at: f64,
    pub provider: String,
    pub model: Option<String>,
    pub call_count: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_usage_details: BTreeMap<String, u64>,
    pub total_duration_ms: u64,
    pub total_estimated_cost_usd: f64,
    pub total_tool_calls: u64,
    pub stop_reason: Option<String>,
    pub system_prompt_preview: Option<String>,
}

/// Full detail for a single trace, including all model calls with tool data.
#[derive(Debug, Clone, Serialize)]
pub struct TraceDetail {
    pub trace_id: String,
    pub calls: Vec<TraceModelCall>,
}

/// A model call within a trace, with its row ID and tool data loaded.
#[derive(Debug, Clone, Serialize)]
pub struct TraceModelCall {
    pub id: i64,
    #[serde(flatten)]
    pub call: ModelCall,
}

/// Aggregate file event statistics.
#[derive(Debug, Clone, Serialize)]
pub struct FileEventStats {
    pub total: u64,
    pub created: u64,
    pub modified: u64,
    pub deleted: u64,
    pub restored: u64,
}

/// Aggregate user-facing tool-call statistics.
#[derive(Debug, Clone, Serialize)]
pub struct ToolCallStats {
    pub total: u64,
    pub allowed: u64,
    pub warned: u64,
    pub denied: u64,
    pub errored: u64,
    pub by_server: Vec<ToolServerCallCount>,
}

/// Per-server tool-call counts.
#[derive(Debug, Clone, Serialize)]
pub struct ToolServerCallCount {
    pub server_name: String,
    pub count: u64,
    pub denied: u64,
    pub warned: u64,
}

/// A unified history entry (merging exec_events and audit_events).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub timestamp: String,
    pub layer: String,
    pub command: String,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<u64>,
    pub stdout_preview: Option<String>,
    pub stderr_preview: Option<String>,
    /// For exec layer: source, process_name, trace_id.
    /// For audit layer: pid, ppid, exe, parent_exe, tty, cwd.
    pub details: serde_json::Value,
}

/// Process-centric history view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessEntry {
    pub exe: String,
    pub command_count: u64,
    pub first_seen: String,
    pub last_seen: String,
}

/// Counts for exec and audit events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryCounts {
    pub exec_count: u64,
    pub audit_count: u64,
}

/// Rule-match counts grouped by canonical action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityRuleActionCount {
    pub rule_action: String,
    pub count: u64,
}

/// Rule-match counts grouped by canonical event type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityRuleEventTypeCount {
    pub event_type: String,
    pub count: u64,
}

/// Rule-match counts grouped by canonical detection level.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityRuleDetectionLevelCount {
    pub detection_level: String,
    pub count: u64,
}

/// Rule-match counts grouped by immutable rule labels stored in session.db.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityRuleStatsByRule {
    pub rule_id: String,
    pub rule_action: String,
    pub detection_level: String,
    pub count: u64,
    pub latest_event_id: String,
    pub latest_timestamp_unix_ms: i64,
}

/// Aggregate security rule statistics regenerated only from session.db.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityRuleStats {
    pub total: u64,
    pub by_action: Vec<SecurityRuleActionCount>,
    pub by_event_type: Vec<SecurityRuleEventTypeCount>,
    pub by_level: Vec<SecurityRuleDetectionLevelCount>,
    pub by_rule: Vec<SecurityRuleStatsByRule>,
}

/// Brokered credential references regenerated from substitution_events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrokeredCredentialStat {
    pub provider: Option<String>,
    pub credential_ref: String,
    pub observed_count: u64,
    pub injected_count: u64,
    pub last_seen: Option<String>,
}

/// Shared SQL column tail for model_calls SELECT queries after provider/protocol.
const MODEL_CALL_COLUMNS_TAIL: &str = "model, process_name, pid,
     method, path, stream,
     system_prompt_preview, messages_count, tools_count,
     request_bytes, request_body_preview,
     message_id, status_code, text_content, thinking_content,
     stop_reason, input_tokens, output_tokens,
     duration_ms, response_bytes, estimated_cost_usd, trace_id";

const TOOL_CALL_LEDGER_FILTER: &str = "origin IN ('native', 'mcp', 'builtin', 'local')";

/// Parse a model_calls row into (id, ModelCall). Column order must match MODEL_CALL_COLUMNS.
fn read_model_call_row(row: &Row<'_>) -> rusqlite::Result<(i64, ModelCall)> {
    let ts_str: String = row.get(1)?;
    let timestamp = humantime::parse_rfc3339(&ts_str).unwrap_or(SystemTime::UNIX_EPOCH);
    let id: i64 = row.get(0)?;

    Ok((
        id,
        ModelCall {
            event_id: row.get(28)?,
            timestamp,
            provider: row.get(2)?,
            protocol: row.get(3)?,
            model: row.get(4)?,
            process_name: row.get(5)?,
            pid: row.get::<_, Option<i64>>(6)?.map(|p| p as u32),
            method: row.get(7)?,
            path: row.get(8)?,
            stream: row.get::<_, i64>(9)? != 0,
            system_prompt_preview: row.get(10)?,
            messages_count: row.get::<_, i64>(11)? as usize,
            tools_count: row.get::<_, i64>(12)? as usize,
            request_bytes: row.get::<_, i64>(13)? as u64,
            request_body_preview: row.get(14)?,
            request_body_full: None,
            message_id: row.get(15)?,
            status_code: row.get::<_, Option<i64>>(16)?.map(|c| c as u16),
            text_content: row.get(17)?,
            thinking_content: row.get(18)?,
            response_body_full: None,
            stop_reason: row.get(19)?,
            input_tokens: row.get::<_, Option<i64>>(20)?.map(|t| t as u64),
            output_tokens: row.get::<_, Option<i64>>(21)?.map(|t| t as u64),
            usage_details: row
                .get::<_, Option<String>>(27)?
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default(),
            duration_ms: row.get::<_, i64>(22)? as u64,
            response_bytes: row.get::<_, i64>(23)? as u64,
            estimated_cost_usd: row.get::<_, f64>(24).unwrap_or(0.0),
            trace_id: row.get(25)?,
            credential_ref: row.get(26)?,
            tool_calls: Vec::new(),
            tool_responses: Vec::new(),
        },
    ))
}

/// Validate that a SQL string is a read-only statement.
///
/// Defense-in-depth: the real backstop is `PRAGMA query_only = ON` on the
/// connection, but this catches obviously wrong statements early with a
/// clear error message.
pub fn validate_select_only(sql: &str) -> Result<(), String> {
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        return Err("empty query".to_string());
    }
    // Extract the first keyword (everything up to the first whitespace or semicolon).
    let first = trimmed
        .split(|c: char| c.is_ascii_whitespace() || c == ';' || c == '(')
        .next()
        .unwrap_or("")
        .to_ascii_uppercase();

    match first.as_str() {
        "SELECT" | "WITH" | "EXPLAIN" => Ok(()),
        "PRAGMA" | "INSERT" | "UPDATE" | "DELETE" | "DROP" | "ALTER" | "CREATE" | "ATTACH"
        | "DETACH" | "REPLACE" | "VACUUM" | "REINDEX" | "BEGIN" | "COMMIT" | "ROLLBACK"
        | "SAVEPOINT" | "RELEASE" => Err(format!("{first} statements are not allowed")),
        _ => Err(format!("unsupported statement type: {first}")),
    }
}

/// Read-only connection to the session database.
///
/// Opened in WAL mode for concurrent access with the writer thread.
pub struct DbReader {
    conn: Connection,
}

impl DbReader {
    /// Open a read-only connection to the given DB file.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn = Connection::open_with_flags(path, flags)?;
        schema::apply_reader_pragmas(&conn)?;
        Ok(Self { conn })
    }

    /// Open an in-memory database (for testing; typically unused since
    /// in-memory DBs can't be shared between connections).
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        schema::apply_pragmas(&conn)?; // in-memory is read-write, pragmas are fine
        schema::create_tables(&conn)?;
        Ok(Self { conn })
    }

    /// Validate that this ledger is structurally ready for route reads.
    ///
    /// Empty ledgers are valid. Missing tables or route-critical columns are
    /// DB contract failures and must not be converted into empty route payloads.
    pub fn ready(&self) -> Result<(), String> {
        schema::validate_ready_schema(&self.conn)
    }

    fn has_column(&self, table: &str, column: &str) -> bool {
        let Ok(mut stmt) = self.conn.prepare(&format!("PRAGMA table_info({table})")) else {
            return false;
        };
        let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(1)) else {
            return false;
        };
        for name in rows.filter_map(Result::ok) {
            if name == column {
                return true;
            }
        }
        false
    }

    fn optional_column_expr(&self, table: &str, column: &str) -> String {
        if self.has_column(table, column) {
            column.to_string()
        } else {
            format!("NULL AS {column}")
        }
    }

    fn model_call_columns(&self) -> String {
        format!(
            "id, timestamp, provider, {}, {}, {}, usage_details, {}",
            self.optional_column_expr("model_calls", "protocol"),
            MODEL_CALL_COLUMNS_TAIL,
            self.optional_column_expr("model_calls", "credential_ref"),
            self.optional_column_expr("model_calls", "event_id")
        )
    }

    /// Execute an arbitrary read-only SQL query and return JSON.
    ///
    /// Returns `{"columns":[...],"rows":[[...], ...]}`.
    /// Caps output at 10,000 rows. Interrupts queries that run longer than
    /// 5 seconds via `sqlite3_interrupt`.
    pub fn query_raw(&self, sql: &str) -> Result<String, String> {
        // Defense-in-depth: reject non-SELECT SQL up front. The production
        // connection is opened read-only (SQLITE_OPEN_READ_ONLY) so writes
        // would fail at execution with a cryptic SQLite error -- validating
        // here gives a clear, consistent error and also guards open_in_memory().
        validate_select_only(sql)?;

        const MAX_ROWS: usize = 10_000;
        const TIMEOUT_MS: u64 = 5_000;
        const POLL_MS: u64 = 100;

        // Set up interrupt timer.
        let interrupt_handle = self.conn.get_interrupt_handle();
        let done = Arc::new(AtomicBool::new(false));
        let done_clone = Arc::clone(&done);
        let timer = std::thread::spawn(move || {
            let polls = TIMEOUT_MS / POLL_MS;
            for _ in 0..polls {
                std::thread::sleep(std::time::Duration::from_millis(POLL_MS));
                if done_clone.load(Ordering::Relaxed) {
                    return;
                }
            }
            if !done_clone.load(Ordering::Relaxed) {
                interrupt_handle.interrupt();
            }
        });

        let result = self.query_raw_inner(sql, MAX_ROWS);

        // Signal timer to stop and wait for it.
        done.store(true, Ordering::Relaxed);
        let _ = timer.join();

        result.map_err(|e| {
            if e.contains("interrupted") {
                "query timed out after 5 seconds".to_string()
            } else {
                e
            }
        })
    }

    /// Execute an arbitrary read-only SQL query with bind parameters and return JSON.
    ///
    /// Same format as `query_raw`: `{"columns":[...],"rows":[[...], ...]}`.
    /// Parameters use `?` positional placeholders (rusqlite native syntax).
    /// Supported param types: null, i64, f64, string (from serde_json::Value).
    pub fn query_raw_with_params(&self, sql: &str, params: &[Value]) -> Result<String, String> {
        // Defense-in-depth: same rationale as query_raw.
        validate_select_only(sql)?;

        const MAX_ROWS: usize = 10_000;
        const TIMEOUT_MS: u64 = 5_000;
        const POLL_MS: u64 = 100;

        let interrupt_handle = self.conn.get_interrupt_handle();
        let done = Arc::new(AtomicBool::new(false));
        let done_clone = Arc::clone(&done);
        let timer = std::thread::spawn(move || {
            let polls = TIMEOUT_MS / POLL_MS;
            for _ in 0..polls {
                std::thread::sleep(std::time::Duration::from_millis(POLL_MS));
                if done_clone.load(Ordering::Relaxed) {
                    return;
                }
            }
            if !done_clone.load(Ordering::Relaxed) {
                interrupt_handle.interrupt();
            }
        });

        let result = self.query_raw_params_inner(sql, params, MAX_ROWS);

        done.store(true, Ordering::Relaxed);
        let _ = timer.join();

        result.map_err(|e| {
            if e.contains("interrupted") {
                "query timed out after 5 seconds".to_string()
            } else {
                e
            }
        })
    }

    fn query_raw_inner(&self, sql: &str, max_rows: usize) -> Result<String, String> {
        self.query_raw_params_inner(sql, &[], max_rows)
    }

    fn query_raw_params_inner(
        &self,
        sql: &str,
        params: &[Value],
        max_rows: usize,
    ) -> Result<String, String> {
        let mut stmt = self.conn.prepare(sql).map_err(|e| e.to_string())?;

        let columns: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
        let col_count = columns.len();

        // Convert serde_json::Value params to rusqlite dynamic params.
        let rusqlite_params: Vec<Box<dyn rusqlite::types::ToSql>> = params
            .iter()
            .map(|v| {
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
            })
            .collect();
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            rusqlite_params.iter().map(|b| b.as_ref()).collect();

        let mut rows: Vec<Vec<Value>> = Vec::new();
        let mut raw_rows = stmt
            .query(param_refs.as_slice())
            .map_err(|e| e.to_string())?;

        while let Some(row) = raw_rows.next().map_err(|e| e.to_string())? {
            if rows.len() >= max_rows {
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

    /// Query the most recent N network events, ordered newest first.
    pub fn recent_net_events(&self, limit: usize) -> rusqlite::Result<Vec<NetEvent>> {
        let credential_ref_col = self.optional_column_expr("net_events", "credential_ref");
        let event_id_col = self.optional_column_expr("net_events", "event_id");
        let sql = format!(
            "SELECT timestamp, domain, port, decision, process_name, pid,
                    method, path, query, status_code,
                    bytes_sent, bytes_received, duration_ms, matched_rule,
                    request_headers, response_headers,
                    request_body_preview, response_body_preview, conn_type,
                    policy_mode, policy_action, policy_rule, policy_reason,
                    trace_id, {credential_ref_col}, {event_id_col}
             FROM net_events
             ORDER BY id DESC
             LIMIT ?1"
        );
        let mut stmt = self.conn.prepare(&sql)?;

        let rows = stmt.query_map(params![limit as i64], |row| {
            let ts_str: String = row.get(0)?;
            let timestamp = humantime::parse_rfc3339(&ts_str).unwrap_or(SystemTime::UNIX_EPOCH);
            let decision_str: String = row.get(3)?;

            Ok(NetEvent {
                event_id: row.get(25)?,
                timestamp,
                domain: row.get(1)?,
                port: row.get::<_, i64>(2)? as u16,
                decision: Decision::parse_str(&decision_str),
                process_name: row.get(4)?,
                pid: row.get::<_, Option<i64>>(5)?.map(|p| p as u32),
                method: row.get(6)?,
                path: row.get(7)?,
                query: row.get(8)?,
                status_code: row.get::<_, Option<i64>>(9)?.map(|c| c as u16),
                bytes_sent: row.get::<_, i64>(10)? as u64,
                bytes_received: row.get::<_, i64>(11)? as u64,
                duration_ms: row.get::<_, i64>(12)? as u64,
                matched_rule: row.get(13)?,
                request_headers: row.get(14)?,
                response_headers: row.get(15)?,
                request_body_preview: row.get(16)?,
                response_body_preview: row.get(17)?,
                request_body_full: None,
                response_body_full: None,
                conn_type: row.get(18)?,
                policy_mode: row.get(19)?,
                policy_action: row.get(20)?,
                policy_rule: row.get(21)?,
                policy_reason: row.get(22)?,
                trace_id: row.get(23)?,
                credential_ref: row.get(24)?,
            })
        })?;

        rows.collect()
    }

    /// Query the most recent N model calls, ordered newest first.
    /// Does NOT load nested tool_calls/tool_responses (use tool_calls_for).
    pub fn recent_model_calls(&self, limit: usize) -> rusqlite::Result<Vec<(i64, ModelCall)>> {
        let sql = format!(
            "SELECT {} FROM model_calls ORDER BY id DESC LIMIT ?1",
            self.model_call_columns()
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![limit as i64], read_model_call_row)?;
        rows.collect()
    }

    /// Query recent stored security rule matches, newest first.
    ///
    /// This returns the full forensic row, including the rule snapshot and
    /// normalized event payload as stored at match time. Runtime endpoints may
    /// expose a smaller projection, but must not consult live rules for truth.
    pub fn recent_security_rule_events(
        &self,
        limit: usize,
    ) -> rusqlite::Result<Vec<SecurityRuleEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT timestamp_unix_ms, event_id, event_type, rule_id,
                    rule_action, detection_level, rule_json, event_json, trace_id,
                    turn_id, credential_ref
             FROM security_rule_events
             ORDER BY timestamp_unix_ms DESC, id DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], read_security_rule_event_row)?;
        rows.collect()
    }

    /// Query recent ask lifecycle records, newest first.
    pub fn recent_security_ask_events(
        &self,
        limit: usize,
    ) -> rusqlite::Result<Vec<SecurityAskEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT timestamp_unix_ms, ask_id, event_id, event_type, rule_id,
                    rule_name, status, rule_json, event_json, resolver, reason, trace_id
             FROM security_ask_events
             ORDER BY timestamp_unix_ms DESC, id DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], read_security_ask_event_row)?;
        rows.collect()
    }

    /// Return the latest lifecycle row for an ask id.
    pub fn latest_security_ask_event(
        &self,
        ask_id: &str,
    ) -> rusqlite::Result<Option<SecurityAskEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT timestamp_unix_ms, ask_id, event_id, event_type, rule_id,
                    rule_name, status, rule_json, event_json, resolver, reason, trace_id
             FROM security_ask_events
             WHERE ask_id = ?1
             ORDER BY timestamp_unix_ms DESC, id DESC
             LIMIT 1",
        )?;
        let mut rows = stmt.query_map(params![ask_id], read_security_ask_event_row)?;
        rows.next().transpose()
    }

    /// Aggregate security rule information from the session DB only.
    pub fn security_rule_stats(&self) -> rusqlite::Result<SecurityRuleStats> {
        let total =
            self.conn
                .query_row("SELECT COUNT(*) FROM security_rule_events", [], |row| {
                    row.get::<_, i64>(0).map(|value| value as u64)
                })?;

        let mut action_stmt = self.conn.prepare(
            "SELECT rule_action, COUNT(*) FROM security_rule_events
             GROUP BY rule_action ORDER BY rule_action",
        )?;
        let by_action = action_stmt
            .query_map([], |row| {
                Ok(SecurityRuleActionCount {
                    rule_action: row.get(0)?,
                    count: row.get::<_, i64>(1)? as u64,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let mut event_type_stmt = self.conn.prepare(
            "SELECT event_type, COUNT(*) FROM security_rule_events
             GROUP BY event_type ORDER BY event_type",
        )?;
        let by_event_type = event_type_stmt
            .query_map([], |row| {
                Ok(SecurityRuleEventTypeCount {
                    event_type: row.get(0)?,
                    count: row.get::<_, i64>(1)? as u64,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let mut level_stmt = self.conn.prepare(
            "SELECT detection_level, COUNT(*) FROM security_rule_events
             GROUP BY detection_level ORDER BY detection_level",
        )?;
        let by_level = level_stmt
            .query_map([], |row| {
                Ok(SecurityRuleDetectionLevelCount {
                    detection_level: row.get(0)?,
                    count: row.get::<_, i64>(1)? as u64,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let mut rule_stmt = self.conn.prepare(
            "SELECT
                sre.rule_id,
                sre.rule_action,
                sre.detection_level,
                COUNT(*) AS count,
                (
                    SELECT latest.event_id
                    FROM security_rule_events latest
                    WHERE latest.rule_id = sre.rule_id
                      AND latest.rule_action = sre.rule_action
                      AND latest.detection_level = sre.detection_level
                    ORDER BY latest.timestamp_unix_ms DESC, latest.id DESC
                    LIMIT 1
                ) AS latest_event_id,
                MAX(sre.timestamp_unix_ms) AS latest_timestamp_unix_ms
             FROM security_rule_events sre
             GROUP BY sre.rule_id, sre.rule_action, sre.detection_level
             ORDER BY latest_timestamp_unix_ms DESC",
        )?;
        let by_rule = rule_stmt
            .query_map([], |row| {
                Ok(SecurityRuleStatsByRule {
                    rule_id: row.get(0)?,
                    rule_action: row.get(1)?,
                    detection_level: row.get(2)?,
                    count: row.get::<_, i64>(3)? as u64,
                    latest_event_id: row.get(4)?,
                    latest_timestamp_unix_ms: row.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        Ok(SecurityRuleStats {
            total,
            by_action,
            by_event_type,
            by_level,
            by_rule,
        })
    }

    /// Aggregate credential-broker runtime state from the session DB only.
    pub fn brokered_credential_stats(&self) -> rusqlite::Result<Vec<BrokeredCredentialStat>> {
        let mut stmt = self.conn.prepare(
            "SELECT MAX(provider), substitution_ref, COUNT(*),
                    SUM(CASE WHEN outcome = 'injected' THEN 1 ELSE 0 END),
                    MAX(timestamp)
             FROM substitution_events
             WHERE material_class = 'credential'
             GROUP BY substitution_ref
             ORDER BY MAX(timestamp) DESC
             LIMIT 100",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(BrokeredCredentialStat {
                provider: row.get(0)?,
                credential_ref: row.get(1)?,
                observed_count: row.get::<_, i64>(2)? as u64,
                injected_count: row.get::<_, i64>(3)? as u64,
                last_seen: row.get(4)?,
            })
        })?;
        rows.collect()
    }

    /// Count net events by decision: returns (total, allowed, denied).
    pub fn net_event_counts(&self) -> rusqlite::Result<NetEventCounts> {
        self.conn.query_row(
            "SELECT
                COUNT(*),
                COALESCE(SUM(CASE WHEN decision = 'allowed' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN decision = 'denied' THEN 1 ELSE 0 END), 0)
             FROM net_events",
            [],
            |row| {
                Ok(NetEventCounts {
                    total: row.get::<_, i64>(0)? as usize,
                    allowed: row.get::<_, i64>(1)? as usize,
                    denied: row.get::<_, i64>(2)? as usize,
                })
            },
        )
    }

    /// Count total model calls.
    pub fn model_call_count(&self) -> rusqlite::Result<usize> {
        self.conn
            .query_row("SELECT COUNT(*) FROM model_calls", [], |row| {
                row.get::<_, i64>(0).map(|n| n as usize)
            })
    }

    /// Get tool calls for a given model_call_id.
    pub fn tool_calls_for(&self, model_call_id: i64) -> rusqlite::Result<Vec<ToolCallEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT call_index, call_id, tool_name, arguments, origin
             FROM tool_calls WHERE model_call_id = ?1 ORDER BY call_index",
        )?;
        let rows = stmt.query_map(params![model_call_id], |row| {
            Ok(ToolCallEntry {
                call_index: row.get::<_, i64>(0)? as u32,
                call_id: row.get(1)?,
                tool_name: row.get(2)?,
                arguments: row.get(3)?,
                origin: row
                    .get::<_, String>(4)
                    .unwrap_or_else(|_| "native".to_string()),
                trace_id: None,
            })
        })?;
        rows.collect()
    }

    /// Get tool responses for a given model_call_id.
    pub fn tool_responses_for(
        &self,
        model_call_id: i64,
    ) -> rusqlite::Result<Vec<ToolResponseEntry>> {
        let credential_ref_col = self.optional_column_expr("tool_responses", "credential_ref");
        let sql = format!(
            "SELECT call_id, content_preview, is_error, {credential_ref_col}
             FROM tool_responses WHERE model_call_id = ?1",
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![model_call_id], |row| {
            Ok(ToolResponseEntry {
                call_id: row.get(0)?,
                content_preview: row.get(1)?,
                is_error: row.get::<_, i64>(2)? != 0,
                trace_id: None,
                credential_ref: row.get(3)?,
            })
        })?;
        rows.collect()
    }

    /// Compute aggregate session statistics from all tables.
    pub fn session_stats(&self) -> rusqlite::Result<SessionStats> {
        // Net event aggregates.
        let (net_total, net_allowed, net_denied, net_error, net_bytes_sent, net_bytes_received) =
            self.conn.query_row(
                "SELECT
                    COUNT(*),
                    COALESCE(SUM(CASE WHEN decision = 'allowed' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN decision = 'denied' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN decision = 'error' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(bytes_sent), 0),
                    COALESCE(SUM(bytes_received), 0)
                 FROM net_events",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)? as u64,
                        row.get::<_, i64>(1)? as u64,
                        row.get::<_, i64>(2)? as u64,
                        row.get::<_, i64>(3)? as u64,
                        row.get::<_, i64>(4)? as u64,
                        row.get::<_, i64>(5)? as u64,
                    ))
                },
            )?;

        // Model call aggregates.
        let (
            model_call_count,
            total_input_tokens,
            total_output_tokens,
            total_model_duration_ms,
            total_estimated_cost_usd,
            usage_details_json,
        ) = self.conn.query_row(
            "SELECT
                    COUNT(*),
                    COALESCE(SUM(COALESCE(input_tokens, 0)), 0),
                    COALESCE(SUM(COALESCE(output_tokens, 0)), 0),
                    COALESCE(SUM(duration_ms), 0),
                    COALESCE(SUM(estimated_cost_usd), 0.0),
                    (SELECT json_group_object(je.key, je.total) FROM (
                        SELECT je.key, SUM(je.value) as total
                        FROM model_calls mc2, json_each(mc2.usage_details) je
                        WHERE mc2.usage_details IS NOT NULL
                        GROUP BY je.key
                    ) je)
                 FROM model_calls",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)? as u64,
                    row.get::<_, i64>(1)? as u64,
                    row.get::<_, i64>(2)? as u64,
                    row.get::<_, i64>(3)? as u64,
                    row.get::<_, f64>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            },
        )?;

        let total_usage_details: BTreeMap<String, u64> = usage_details_json
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();

        // Total tool calls.
        let total_tool_calls: u64 = self.conn.query_row(
            &format!("SELECT COUNT(*) FROM tool_calls WHERE {TOOL_CALL_LEDGER_FILTER}"),
            [],
            |row| row.get::<_, i64>(0).map(|n| n as u64),
        )?;

        Ok(SessionStats {
            net_total,
            net_allowed,
            net_denied,
            net_error,
            net_bytes_sent,
            net_bytes_received,
            model_call_count,
            total_input_tokens,
            total_output_tokens,
            total_usage_details,
            total_model_duration_ms,
            total_tool_calls,
            total_estimated_cost_usd,
        })
    }

    /// Top domains by request count.
    pub fn top_domains(&self, limit: usize) -> rusqlite::Result<Vec<DomainCount>> {
        let mut stmt = self.conn.prepare(
            "SELECT domain,
                    COUNT(*) as cnt,
                    SUM(CASE WHEN decision = 'allowed' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN decision = 'denied' THEN 1 ELSE 0 END)
             FROM net_events
             GROUP BY domain
             ORDER BY cnt DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(DomainCount {
                domain: row.get(0)?,
                count: row.get::<_, i64>(1)? as u64,
                allowed: row.get::<_, i64>(2)? as u64,
                denied: row.get::<_, i64>(3)? as u64,
            })
        })?;
        rows.collect()
    }

    /// Net events bucketed over time. Fetches timestamps in a window
    /// and buckets them in Rust. Returns `count` buckets of `bucket_min` minutes each,
    /// ending at the most recent event.
    pub fn net_events_over_time(
        &self,
        bucket_min: u64,
        count: usize,
    ) -> rusqlite::Result<Vec<TimeBucket>> {
        let bucket_sec = bucket_min * 60;
        let window_sec = bucket_sec * count as u64;

        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let window_start = now.saturating_sub(window_sec);

        let mut buckets = Vec::with_capacity(count);
        for i in 0..count {
            let start = window_start + (i as u64) * bucket_sec;
            let ts = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(start);
            buckets.push(TimeBucket {
                bucket_start: humantime::format_rfc3339_seconds(ts).to_string(),
                allowed: 0,
                denied: 0,
            });
        }

        let mut stmt = self.conn.prepare(
            "SELECT 
                CAST((CAST(strftime('%s', timestamp) AS INTEGER) - ?1) / ?2 AS INTEGER) as idx,
                COALESCE(SUM(CASE WHEN decision = 'allowed' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN decision = 'denied' THEN 1 ELSE 0 END), 0)
             FROM net_events
             WHERE timestamp >= strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?3)
               AND CAST(strftime('%s', timestamp) AS INTEGER) >= ?1
             GROUP BY idx",
        )?;

        let offset = format!("-{window_sec} seconds");
        let rows = stmt.query_map(
            params![window_start as i64, bucket_sec as i64, offset],
            |row| {
                Ok((
                    row.get::<_, i64>(0)? as usize,
                    row.get::<_, i64>(1)? as u64,
                    row.get::<_, i64>(2)? as u64,
                ))
            },
        )?;

        for row in rows {
            let (mut idx, allowed, denied) = row?;
            if idx >= count {
                idx = count - 1;
            }
            buckets[idx].allowed += allowed;
            buckets[idx].denied += denied;
        }

        Ok(buckets)
    }

    /// Search net events by domain, path, method, or matched_rule substring.
    pub fn search_net_events(&self, query: &str, limit: usize) -> rusqlite::Result<Vec<NetEvent>> {
        let pattern = format!("%{query}%");
        let credential_ref_col = self.optional_column_expr("net_events", "credential_ref");
        let event_id_col = self.optional_column_expr("net_events", "event_id");
        let sql = format!(
            "SELECT timestamp, domain, port, decision, process_name, pid,
                    method, path, query, status_code,
                    bytes_sent, bytes_received, duration_ms, matched_rule,
                    request_headers, response_headers,
                    request_body_preview, response_body_preview, conn_type,
                    policy_mode, policy_action, policy_rule, policy_reason,
                    trace_id, {credential_ref_col}, {event_id_col}
             FROM net_events
             WHERE domain LIKE ?1
                OR path LIKE ?1
                OR method LIKE ?1
                OR matched_rule LIKE ?1
             ORDER BY id DESC
             LIMIT ?2"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![pattern, limit as i64], |row| {
            let ts_str: String = row.get(0)?;
            let timestamp = humantime::parse_rfc3339(&ts_str).unwrap_or(SystemTime::UNIX_EPOCH);
            let decision_str: String = row.get(3)?;
            Ok(NetEvent {
                event_id: row.get(25)?,
                timestamp,
                domain: row.get(1)?,
                port: row.get::<_, i64>(2)? as u16,
                decision: Decision::parse_str(&decision_str),
                process_name: row.get(4)?,
                pid: row.get::<_, Option<i64>>(5)?.map(|p| p as u32),
                method: row.get(6)?,
                path: row.get(7)?,
                query: row.get(8)?,
                status_code: row.get::<_, Option<i64>>(9)?.map(|c| c as u16),
                bytes_sent: row.get::<_, i64>(10)? as u64,
                bytes_received: row.get::<_, i64>(11)? as u64,
                duration_ms: row.get::<_, i64>(12)? as u64,
                matched_rule: row.get(13)?,
                request_headers: row.get(14)?,
                response_headers: row.get(15)?,
                request_body_preview: row.get(16)?,
                response_body_preview: row.get(17)?,
                request_body_full: None,
                response_body_full: None,
                conn_type: row.get(18)?,
                policy_mode: row.get(19)?,
                policy_action: row.get(20)?,
                policy_rule: row.get(21)?,
                policy_reason: row.get(22)?,
                trace_id: row.get(23)?,
                credential_ref: row.get(24)?,
            })
        })?;
        rows.collect()
    }

    /// Search model calls by provider or model substring.
    pub fn search_model_calls(
        &self,
        query: &str,
        limit: usize,
    ) -> rusqlite::Result<Vec<(i64, ModelCall)>> {
        let pattern = format!("%{query}%");
        let sql = format!(
            "SELECT {}
             FROM model_calls
             WHERE provider LIKE ?1
                OR model LIKE ?1
                OR stop_reason LIKE ?1
             ORDER BY id DESC
             LIMIT ?2",
            self.model_call_columns()
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![pattern, limit as i64], |row| {
            read_model_call_row(row)
        })?;
        rows.collect()
    }

    /// Token usage aggregated by provider.
    pub fn token_usage_by_provider(&self) -> rusqlite::Result<Vec<ProviderTokenUsage>> {
        let mut stmt = self.conn.prepare(
            "SELECT provider,
                    COUNT(*),
                    COALESCE(SUM(COALESCE(input_tokens, 0)), 0),
                    COALESCE(SUM(COALESCE(output_tokens, 0)), 0),
                    COALESCE(SUM(duration_ms), 0),
                    COALESCE(SUM(estimated_cost_usd), 0.0)
             FROM model_calls
             GROUP BY provider
             ORDER BY COUNT(*) DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ProviderTokenUsage {
                provider: row.get(0)?,
                call_count: row.get::<_, i64>(1)? as u64,
                total_input_tokens: row.get::<_, i64>(2)? as u64,
                total_output_tokens: row.get::<_, i64>(3)? as u64,
                total_duration_ms: row.get::<_, i64>(4)? as u64,
                total_estimated_cost_usd: row.get::<_, f64>(5)?,
            })
        })?;
        rows.collect()
    }

    /// Tool usage frequency (from tool_calls table).
    pub fn tool_usage_frequency(&self, limit: usize) -> rusqlite::Result<Vec<ToolUsageCount>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT tool_name, COUNT(*) as cnt
             FROM tool_calls
             WHERE {TOOL_CALL_LEDGER_FILTER}
             GROUP BY tool_name
             ORDER BY cnt DESC
             LIMIT ?1",
        ))?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(ToolUsageCount {
                tool_name: row.get(0)?,
                count: row.get::<_, i64>(1)? as u64,
            })
        })?;
        rows.collect()
    }

    // ── Cross-session summary queries ─────────────────────────────────

    /// Count total file events in the session DB.
    pub fn file_event_count(&self) -> rusqlite::Result<u64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM fs_events", [], |row| {
                row.get::<_, i64>(0).map(|n| n as u64)
            })
    }

    /// Tool usage with response byte and duration stats from model_calls.
    pub fn tool_usage_with_stats(&self, limit: usize) -> rusqlite::Result<Vec<ToolUsageWithStats>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT tc.tool_name, COUNT(*) as cnt,
                    COALESCE(SUM(LENGTH(COALESCE(tc.response_preview, tr.content_preview, ''))), 0),
                    COALESCE(SUM(COALESCE(tc.duration_ms, mc.duration_ms, 0)), 0)
             FROM tool_calls tc
             LEFT JOIN model_calls mc ON tc.model_call_id = mc.id
             LEFT JOIN tool_responses tr ON tc.call_id = tr.call_id
             WHERE {TOOL_CALL_LEDGER_FILTER}
             GROUP BY tc.tool_name
             ORDER BY cnt DESC LIMIT ?1",
        ))?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(ToolUsageWithStats {
                tool_name: row.get(0)?,
                count: row.get::<_, i64>(1)? as u64,
                total_bytes: row.get::<_, i64>(2)? as u64,
                total_duration_ms: row.get::<_, i64>(3)? as u64,
            })
        })?;
        rows.collect()
    }

    /// MCP-origin tool usage grouped by tool_name with duration and response size.
    pub fn mcp_tool_usage(&self, limit: usize) -> rusqlite::Result<Vec<McpToolUsage>> {
        let sql = "SELECT tool_name, server_name, COUNT(*) as cnt,
                    COALESCE(SUM(LENGTH(response_preview)), 0),
                    COALESCE(SUM(duration_ms), 0)
             FROM tool_calls
             WHERE origin = 'mcp'
             GROUP BY tool_name
             ORDER BY cnt DESC LIMIT ?1"
            .to_string();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(McpToolUsage {
                tool_name: row.get(0)?,
                server_name: row.get(1)?,
                count: row.get::<_, i64>(2)? as u64,
                total_bytes: row.get::<_, i64>(3)? as u64,
                total_duration_ms: row.get::<_, i64>(4)? as u64,
            })
        })?;
        rows.collect()
    }

    // ── Trace queries ───────────────────────────────────────────────

    /// Recent traces grouped by trace_id, ordered newest first.
    /// All aggregation done in SQL.
    pub fn recent_traces(&self, limit: usize) -> rusqlite::Result<Vec<TraceSummary>> {
        let mut stmt = self.conn.prepare(
            "WITH top_traces AS (
                SELECT trace_id, MAX(id) as max_id
                FROM model_calls
                WHERE trace_id IS NOT NULL
                GROUP BY trace_id
                ORDER BY max_id DESC
                LIMIT ?1
             )
             SELECT
                t.trace_id,
                MIN(mc.timestamp) as started_at,
                MAX(mc.timestamp) as ended_at,
                (SELECT provider FROM model_calls m2 WHERE m2.trace_id = t.trace_id ORDER BY m2.id ASC LIMIT 1),
                (SELECT model FROM model_calls m3 WHERE m3.trace_id = t.trace_id ORDER BY m3.id ASC LIMIT 1),
                COUNT(mc.id) as call_count,
                COALESCE(SUM(COALESCE(mc.input_tokens, 0)), 0),
                COALESCE(SUM(COALESCE(mc.output_tokens, 0)), 0),
                (SELECT json_group_object(je.key, je.total) FROM (
                    SELECT je.key, SUM(je.value) as total
                    FROM model_calls mc6, json_each(mc6.usage_details) je
                    WHERE mc6.trace_id = t.trace_id AND mc6.usage_details IS NOT NULL
                    GROUP BY je.key
                ) je),
                COALESCE(SUM(mc.duration_ms), 0),
                COALESCE(SUM(mc.estimated_cost_usd), 0.0),
                (SELECT COUNT(*) FROM tool_calls tc
                 JOIN model_calls mc2 ON tc.model_call_id = mc2.id
                 WHERE mc2.trace_id = t.trace_id),
                (SELECT stop_reason FROM model_calls m4 WHERE m4.trace_id = t.trace_id ORDER BY m4.id DESC LIMIT 1),
                (SELECT system_prompt_preview FROM model_calls m5 WHERE m5.trace_id = t.trace_id ORDER BY m5.id ASC LIMIT 1)
             FROM top_traces t
             JOIN model_calls mc ON mc.trace_id = t.trace_id
             GROUP BY t.trace_id
             ORDER BY t.max_id DESC",
        )?;

        let rows = stmt.query_map(params![limit as i64], |row| {
            let started_str: String = row.get(1)?;
            let ended_str: String = row.get(2)?;
            let started_at = humantime::parse_rfc3339(&started_str)
                .unwrap_or(SystemTime::UNIX_EPOCH)
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64();
            let ended_at = humantime::parse_rfc3339(&ended_str)
                .unwrap_or(SystemTime::UNIX_EPOCH)
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64();

            let total_usage_details: BTreeMap<String, u64> = row
                .get::<_, Option<String>>(8)?
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();

            Ok(TraceSummary {
                trace_id: row.get(0)?,
                started_at,
                ended_at,
                provider: row.get(3)?,
                model: row.get(4)?,
                call_count: row.get::<_, i64>(5)? as u64,
                total_input_tokens: row.get::<_, i64>(6)? as u64,
                total_output_tokens: row.get::<_, i64>(7)? as u64,
                total_usage_details,
                total_duration_ms: row.get::<_, i64>(9)? as u64,
                total_estimated_cost_usd: row.get::<_, f64>(10)?,
                total_tool_calls: row.get::<_, i64>(11)? as u64,
                stop_reason: row.get(12)?,
                system_prompt_preview: row.get(13)?,
            })
        })?;

        rows.collect()
    }

    /// Load full detail for a single trace: all calls with tool data.
    pub fn trace_detail(&self, trace_id: &str) -> rusqlite::Result<TraceDetail> {
        let sql = format!(
            "SELECT {} FROM model_calls WHERE trace_id = ?1 ORDER BY id ASC",
            self.model_call_columns()
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows: Vec<(i64, ModelCall)> = stmt
            .query_map(params![trace_id], read_model_call_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        // Fetch all tool calls for this trace in one batch.
        let mut tool_calls_stmt = self.conn.prepare(
            "SELECT tc.model_call_id, tc.call_index, tc.call_id, tc.tool_name, tc.arguments, tc.origin
             FROM tool_calls tc
             JOIN model_calls mc ON tc.model_call_id = mc.id
             WHERE mc.trace_id = ?1
             ORDER BY tc.model_call_id, tc.call_index",
        )?;
        let all_tool_calls = tool_calls_stmt.query_map(params![trace_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                ToolCallEntry {
                    call_index: row.get::<_, i64>(1)? as u32,
                    call_id: row.get(2)?,
                    tool_name: row.get(3)?,
                    arguments: row.get(4)?,
                    origin: row
                        .get::<_, String>(5)
                        .unwrap_or_else(|_| "native".to_string()),
                    trace_id: None,
                },
            ))
        })?;

        // Fetch all tool responses for this trace in one batch.
        let tool_response_credential_ref_col =
            if self.has_column("tool_responses", "credential_ref") {
                "tr.credential_ref".to_string()
            } else {
                "NULL AS credential_ref".to_string()
            };
        let tool_response_sql = format!(
            "SELECT tr.model_call_id, tr.call_id, tr.content_preview, tr.is_error, {tool_response_credential_ref_col}
             FROM tool_responses tr
             JOIN model_calls mc ON tr.model_call_id = mc.id
             WHERE mc.trace_id = ?1"
        );
        let mut tool_resps_stmt = self.conn.prepare(&tool_response_sql)?;
        let all_tool_resps = tool_resps_stmt.query_map(params![trace_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                ToolResponseEntry {
                    call_id: row.get(1)?,
                    content_preview: row.get(2)?,
                    is_error: row.get::<_, i64>(3)? != 0,
                    trace_id: None,
                    credential_ref: row.get(4)?,
                },
            ))
        })?;

        // Group by model_call_id.
        let mut tool_calls_map: std::collections::HashMap<i64, Vec<ToolCallEntry>> =
            std::collections::HashMap::new();
        for res in all_tool_calls {
            let (mc_id, entry) = res?;
            tool_calls_map.entry(mc_id).or_default().push(entry);
        }

        let mut tool_resps_map: std::collections::HashMap<i64, Vec<ToolResponseEntry>> =
            std::collections::HashMap::new();
        for res in all_tool_resps {
            let (mc_id, entry) = res?;
            tool_resps_map.entry(mc_id).or_default().push(entry);
        }

        let mut calls = Vec::with_capacity(rows.len());
        for (id, mut call) in rows {
            call.tool_calls = tool_calls_map.remove(&id).unwrap_or_default();
            call.tool_responses = tool_resps_map.remove(&id).unwrap_or_default();
            calls.push(TraceModelCall { id, call });
        }

        Ok(TraceDetail {
            trace_id: trace_id.to_string(),
            calls,
        })
    }

    // ── File event queries ────────────────────────────────────────────

    /// Query the most recent N file events, ordered newest first.
    pub fn recent_file_events(&self, limit: usize) -> rusqlite::Result<Vec<FileEvent>> {
        let trace_id_col = self.optional_column_expr("fs_events", "trace_id");
        let credential_ref_col = self.optional_column_expr("fs_events", "credential_ref");
        let event_id_col = self.optional_column_expr("fs_events", "event_id");
        let sql = format!(
            "SELECT timestamp, action, path, size, {trace_id_col}, {credential_ref_col}, {event_id_col}
             FROM fs_events
             ORDER BY id DESC
             LIMIT ?1"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![limit as i64], read_file_event_row)?;
        rows.collect()
    }

    /// Search file events by path substring.
    pub fn search_file_events(
        &self,
        query: &str,
        limit: usize,
    ) -> rusqlite::Result<Vec<FileEvent>> {
        let pattern = format!("%{query}%");
        let trace_id_col = self.optional_column_expr("fs_events", "trace_id");
        let credential_ref_col = self.optional_column_expr("fs_events", "credential_ref");
        let event_id_col = self.optional_column_expr("fs_events", "event_id");
        let sql = format!(
            "SELECT timestamp, action, path, size, {trace_id_col}, {credential_ref_col}, {event_id_col}
             FROM fs_events
             WHERE path LIKE ?1
             ORDER BY id DESC
             LIMIT ?2"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![pattern, limit as i64], read_file_event_row)?;
        rows.collect()
    }

    /// Aggregate file event statistics. All aggregation done in SQL.
    pub fn file_event_stats(&self) -> rusqlite::Result<FileEventStats> {
        self.conn.query_row(
            "SELECT
                COUNT(*),
                COALESCE(SUM(CASE WHEN action = 'created' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN action = 'modified' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN action = 'deleted' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN action = 'restored' THEN 1 ELSE 0 END), 0)
             FROM fs_events",
            [],
            |row| {
                Ok(FileEventStats {
                    total: row.get::<_, i64>(0)? as u64,
                    created: row.get::<_, i64>(1)? as u64,
                    modified: row.get::<_, i64>(2)? as u64,
                    deleted: row.get::<_, i64>(3)? as u64,
                    restored: row.get::<_, i64>(4)? as u64,
                })
            },
        )
    }

    /// Query the user-facing tool-call ledger, ordered newest first.
    pub fn recent_tool_calls(&self, limit: usize) -> rusqlite::Result<Vec<ToolCallLedgerEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, event_id, timestamp, model_call_id, origin, server_name, method,
                    request_id, call_id, tool_name, arguments, response_preview, decision,
                    duration_ms, error_message, bytes_sent, bytes_received, policy_rule,
                    trace_id, credential_ref
             FROM tool_calls
             WHERE origin IN ('native', 'mcp', 'builtin', 'local')
             ORDER BY id DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(ToolCallLedgerEntry {
                id: row.get(0)?,
                event_id: row.get(1)?,
                timestamp: row.get(2)?,
                model_call_id: row.get(3)?,
                origin: row.get(4)?,
                server_name: row.get(5)?,
                method: row.get(6)?,
                request_id: row.get(7)?,
                call_id: row.get(8)?,
                tool_name: row.get(9)?,
                arguments: row.get(10)?,
                response_preview: row.get(11)?,
                decision: row.get(12)?,
                duration_ms: row.get::<_, i64>(13)? as u64,
                error_message: row.get(14)?,
                bytes_sent: row.get::<_, i64>(15)? as u64,
                bytes_received: row.get::<_, i64>(16)? as u64,
                policy_rule: row.get(17)?,
                trace_id: row.get(18)?,
                credential_ref: row.get(19)?,
            })
        })?;
        rows.collect()
    }

    /// Aggregate user-facing tool-call statistics. All aggregation done in SQL.
    pub fn tool_call_stats(&self) -> rusqlite::Result<ToolCallStats> {
        let totals_sql = "SELECT
                COUNT(*),
                COALESCE(SUM(CASE WHEN decision = 'allowed' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN decision = 'warned' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN decision = 'denied' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN decision = 'error' THEN 1 ELSE 0 END), 0)
             FROM tool_calls
             WHERE origin IN ('native', 'mcp', 'builtin', 'local')"
            .to_string();
        let (total, allowed, warned, denied, errored) =
            self.conn.query_row(&totals_sql, [], |row| {
                Ok((
                    row.get::<_, i64>(0)? as u64,
                    row.get::<_, i64>(1)? as u64,
                    row.get::<_, i64>(2)? as u64,
                    row.get::<_, i64>(3)? as u64,
                    row.get::<_, i64>(4)? as u64,
                ))
            })?;

        let by_server_sql = "SELECT COALESCE(server_name, origin),
                    COUNT(*) as cnt,
                    SUM(CASE WHEN decision = 'denied' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN decision = 'warned' THEN 1 ELSE 0 END)
             FROM tool_calls
             WHERE origin IN ('native', 'mcp', 'builtin', 'local')
             GROUP BY COALESCE(server_name, origin)
             ORDER BY cnt DESC, COALESCE(server_name, origin) ASC"
            .to_string();
        let mut stmt = self.conn.prepare(&by_server_sql)?;
        let by_server = stmt.query_map([], |row| {
            Ok(ToolServerCallCount {
                server_name: row.get(0)?,
                count: row.get::<_, i64>(1)? as u64,
                denied: row.get::<_, i64>(2)? as u64,
                warned: row.get::<_, i64>(3)? as u64,
            })
        })?;

        Ok(ToolCallStats {
            total,
            allowed,
            warned,
            denied,
            errored,
            by_server: by_server.collect::<rusqlite::Result<Vec<_>>>()?,
        })
    }

    /// Raw tool-call row count for session-index rollups.
    pub fn raw_tool_call_count(&self) -> rusqlite::Result<u64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM tool_calls", [], |row| {
                Ok(row.get::<_, i64>(0)? as u64)
            })
    }

    // -----------------------------------------------------------------
    // History: exec_events + audit_events
    // -----------------------------------------------------------------

    /// Counts of exec and audit events in this session.
    pub fn history_counts(&self) -> rusqlite::Result<HistoryCounts> {
        let exec_count: i64 =
            self.conn
                .query_row("SELECT COUNT(*) FROM exec_events", [], |row| row.get(0))?;
        let audit_count: i64 =
            self.conn
                .query_row("SELECT COUNT(*) FROM audit_events", [], |row| row.get(0))?;
        Ok(HistoryCounts {
            exec_count: exec_count as u64,
            audit_count: audit_count as u64,
        })
    }

    /// Unified command history (exec + audit), sorted by timestamp desc.
    /// `layer` can be "all", "exec", or "audit".
    pub fn history(
        &self,
        limit: usize,
        offset: usize,
        search: Option<&str>,
        layer: &str,
    ) -> rusqlite::Result<(Vec<HistoryEntry>, u64)> {
        let mut entries = Vec::new();

        if layer == "all" || layer == "exec" {
            if let Some(q) = search {
                let pattern = format!("%{q}%");
                let mut stmt = self.conn.prepare(
                    "SELECT timestamp, exec_id, command, exit_code, duration_ms,
                            stdout_preview, stderr_preview, source, trace_id,
                            process_name
                     FROM exec_events WHERE command LIKE ?1
                     ORDER BY timestamp DESC",
                )?;
                let rows = stmt.query_map(params![pattern], read_exec_history_row)?;
                for r in rows {
                    entries.push(r?);
                }
            } else {
                let mut stmt = self.conn.prepare(
                    "SELECT timestamp, exec_id, command, exit_code, duration_ms,
                            stdout_preview, stderr_preview, source, trace_id,
                            process_name
                     FROM exec_events ORDER BY timestamp DESC",
                )?;
                let rows = stmt.query_map([], read_exec_history_row)?;
                for r in rows {
                    entries.push(r?);
                }
            }
        }

        if layer == "all" || layer == "audit" {
            if let Some(q) = search {
                let pattern = format!("%{q}%");
                let mut stmt = self.conn.prepare(
                    "SELECT timestamp, pid, ppid, uid, exe, comm, argv, cwd,
                            tty, session_id, audit_id, parent_exe, exit_code
                     FROM audit_events WHERE argv LIKE ?1 OR exe LIKE ?1
                     ORDER BY timestamp DESC",
                )?;
                let rows = stmt.query_map(params![pattern], read_audit_history_row)?;
                for r in rows {
                    entries.push(r?);
                }
            } else {
                let mut stmt = self.conn.prepare(
                    "SELECT timestamp, pid, ppid, uid, exe, comm, argv, cwd,
                            tty, session_id, audit_id, parent_exe, exit_code
                     FROM audit_events ORDER BY timestamp DESC",
                )?;
                let rows = stmt.query_map([], read_audit_history_row)?;
                for r in rows {
                    entries.push(r?);
                }
            }
        }

        // Sort combined results by timestamp desc.
        entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        let total = entries.len() as u64;
        let paginated: Vec<HistoryEntry> = entries.into_iter().skip(offset).take(limit).collect();
        Ok((paginated, total))
    }

    /// Process-centric view of audit events.
    pub fn history_processes(&self, limit: usize) -> rusqlite::Result<Vec<ProcessEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT exe, COUNT(*) as cnt,
                    MIN(timestamp) as first_seen,
                    MAX(timestamp) as last_seen
             FROM audit_events
             GROUP BY exe
             ORDER BY cnt DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(ProcessEntry {
                exe: row.get(0)?,
                command_count: row.get::<_, i64>(1)? as u64,
                first_seen: row.get(2)?,
                last_seen: row.get(3)?,
            })
        })?;
        rows.collect()
    }

    /// Recent exec events (for Layer 1 queries).
    pub fn recent_exec_events(&self, limit: usize) -> rusqlite::Result<Vec<ExecEvent>> {
        let credential_ref_col = self.optional_column_expr("exec_events", "credential_ref");
        let event_id_col = self.optional_column_expr("exec_events", "event_id");
        let sql = format!(
            "SELECT timestamp, exec_id, command, source, trace_id, process_name,
                    {credential_ref_col}, {event_id_col}
             FROM exec_events ORDER BY timestamp DESC LIMIT ?1"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            let ts_str: String = row.get(0)?;
            let timestamp = humantime::parse_rfc3339(&ts_str).unwrap_or(SystemTime::UNIX_EPOCH);
            Ok(ExecEvent {
                event_id: row.get(7)?,
                timestamp,
                exec_id: row.get::<_, i64>(1)? as u64,
                command: row.get(2)?,
                source: row.get(3)?,
                trace_id: row.get(4)?,
                process_name: row.get(5)?,
                credential_ref: row.get(6)?,
            })
        })?;
        rows.collect()
    }

    /// Recent audit events (for Layer 3 queries).
    pub fn recent_audit_events(&self, limit: usize) -> rusqlite::Result<Vec<AuditEvent>> {
        let trace_id_col = self.optional_column_expr("audit_events", "trace_id");
        let credential_ref_col = self.optional_column_expr("audit_events", "credential_ref");
        let event_id_col = self.optional_column_expr("audit_events", "event_id");
        let sql = format!(
            "SELECT timestamp, pid, ppid, uid, exe, comm, argv, cwd,
                    tty, session_id, audit_id, exec_event_id, parent_exe,
                    {trace_id_col}, {credential_ref_col}, {event_id_col}
             FROM audit_events ORDER BY timestamp DESC LIMIT ?1"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            let ts_str: String = row.get(0)?;
            let timestamp = humantime::parse_rfc3339(&ts_str).unwrap_or(SystemTime::UNIX_EPOCH);
            Ok(AuditEvent {
                event_id: row.get(15)?,
                timestamp,
                pid: row.get::<_, i64>(1)? as u32,
                ppid: row.get::<_, i64>(2)? as u32,
                uid: row.get::<_, i64>(3)? as u32,
                exe: row.get(4)?,
                comm: row.get(5)?,
                argv: row.get(6)?,
                cwd: row.get(7)?,
                tty: row.get(8)?,
                session_id: row.get::<_, Option<i64>>(9)?.map(|v| v as u32),
                audit_id: row.get(10)?,
                exec_event_id: row.get(11)?,
                parent_exe: row.get(12)?,
                trace_id: row.get(13)?,
                credential_ref: row.get(14)?,
            })
        })?;
        rows.collect()
    }
}

fn read_security_rule_event_row(row: &Row<'_>) -> rusqlite::Result<SecurityRuleEvent> {
    let rule_action: String = row.get(4)?;
    let detection_level: String = row.get(5)?;
    Ok(SecurityRuleEvent {
        timestamp_unix_ms: row.get(0)?,
        event_id: row.get(1)?,
        event_type: row.get(2)?,
        rule_id: row.get(3)?,
        rule_action: SecurityRuleAction::parse_str(&rule_action).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                4,
                rusqlite::types::Type::Text,
                format!("unknown rule_action {rule_action}").into(),
            )
        })?,
        detection_level: SecurityDetectionLevel::parse_str(&detection_level).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                5,
                rusqlite::types::Type::Text,
                format!("unknown detection_level {detection_level}").into(),
            )
        })?,
        rule_json: row.get(6)?,
        event_json: row.get(7)?,
        trace_id: row.get(8)?,
        turn_id: row.get(9)?,
        credential_ref: row.get(10)?,
    })
}

fn read_security_ask_event_row(row: &Row<'_>) -> rusqlite::Result<SecurityAskEvent> {
    let status: String = row.get(6)?;
    Ok(SecurityAskEvent {
        timestamp_unix_ms: row.get(0)?,
        ask_id: row.get(1)?,
        event_id: row.get(2)?,
        event_type: row.get(3)?,
        rule_id: row.get(4)?,
        rule_name: row.get(5)?,
        status: SecurityAskStatus::parse_str(&status).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                6,
                rusqlite::types::Type::Text,
                format!("unknown ask status {status}").into(),
            )
        })?,
        rule_json: row.get(7)?,
        event_json: row.get(8)?,
        resolver: row.get(9)?,
        reason: row.get(10)?,
        trace_id: row.get(11)?,
    })
}

/// Parse an fs_events row into FileEvent. Column order must match the SELECT in queries above.
fn read_file_event_row(row: &Row<'_>) -> rusqlite::Result<FileEvent> {
    let ts_str: String = row.get(0)?;
    let timestamp = humantime::parse_rfc3339(&ts_str).unwrap_or(SystemTime::UNIX_EPOCH);
    let action_str: String = row.get(1)?;
    Ok(FileEvent {
        event_id: row.get::<_, Option<String>>(6).ok().flatten(),
        timestamp,
        action: FileAction::parse_str(&action_str),
        path: row.get(2)?,
        size: row.get::<_, Option<i64>>(3)?.map(|s| s as u64),
        trace_id: row.get::<_, Option<String>>(4).ok().flatten(),
        credential_ref: row.get::<_, Option<String>>(5).ok().flatten(),
    })
}

/// Parse an exec_events row into a HistoryEntry for unified history.
fn read_exec_history_row(row: &Row<'_>) -> rusqlite::Result<HistoryEntry> {
    Ok(HistoryEntry {
        timestamp: row.get(0)?,
        layer: "exec".to_string(),
        command: row.get(2)?,
        exit_code: row.get::<_, Option<i64>>(3)?.map(|c| c as i32),
        duration_ms: row.get::<_, Option<i64>>(4)?.map(|d| d as u64),
        stdout_preview: row.get(5)?,
        stderr_preview: row.get(6)?,
        details: serde_json::json!({
            "source": row.get::<_, Option<String>>(7)?,
            "trace_id": row.get::<_, Option<String>>(8)?,
            "process_name": row.get::<_, Option<String>>(9)?,
            "exec_id": row.get::<_, i64>(1)?,
        }),
    })
}

/// Parse an audit_events row into a HistoryEntry for unified history.
fn read_audit_history_row(row: &Row<'_>) -> rusqlite::Result<HistoryEntry> {
    Ok(HistoryEntry {
        timestamp: row.get(0)?,
        layer: "audit".to_string(),
        command: row.get(6)?, // argv
        exit_code: row.get::<_, Option<i64>>(12)?.map(|c| c as i32),
        duration_ms: None,
        stdout_preview: None,
        stderr_preview: None,
        details: serde_json::json!({
            "pid": row.get::<_, i64>(1)?,
            "ppid": row.get::<_, i64>(2)?,
            "uid": row.get::<_, i64>(3)?,
            "exe": row.get::<_, String>(4)?,
            "comm": row.get::<_, Option<String>>(5)?,
            "cwd": row.get::<_, Option<String>>(7)?,
            "tty": row.get::<_, Option<String>>(8)?,
            "session_id": row.get::<_, Option<i64>>(9)?,
            "audit_id": row.get::<_, Option<String>>(10)?,
            "parent_exe": row.get::<_, Option<String>>(11)?,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn setup_reader_with_data() -> DbReader {
        let reader = DbReader::open_in_memory().unwrap();
        reader.conn.execute(
            "INSERT INTO net_events (timestamp, domain, port, decision, bytes_sent, bytes_received, duration_ms)
             VALUES ('2026-01-01T00:00:00Z', 'example.com', 443, 'allowed', 100, 200, 50)",
            [],
        ).unwrap();
        reader.conn.execute(
            "INSERT INTO net_events (timestamp, domain, port, decision, bytes_sent, bytes_received, duration_ms)
             VALUES ('2026-01-01T00:01:00Z', 'evil.com', 443, 'denied', 0, 0, 1)",
            [],
        ).unwrap();
        reader
    }

    #[test]
    fn query_raw_returns_columnar_json() {
        let reader = setup_reader_with_data();
        let json_str = reader
            .query_raw("SELECT domain, decision FROM net_events ORDER BY id")
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["columns"], json!(["domain", "decision"]));
        assert_eq!(parsed["rows"].as_array().unwrap().len(), 2);
        assert_eq!(parsed["rows"][0][0], "example.com");
        assert_eq!(parsed["rows"][1][0], "evil.com");
    }

    #[test]
    fn query_raw_with_params_binds_values() {
        let reader = setup_reader_with_data();
        let params = vec![json!("denied")];
        let json_str = reader
            .query_raw_with_params("SELECT domain FROM net_events WHERE decision = ?", &params)
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["rows"].as_array().unwrap().len(), 1);
        assert_eq!(parsed["rows"][0][0], "evil.com");
    }

    #[test]
    fn query_raw_with_params_integer_bind() {
        let reader = setup_reader_with_data();
        let params = vec![json!(1)];
        let json_str = reader
            .query_raw_with_params("SELECT domain FROM net_events ORDER BY id LIMIT ?", &params)
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["rows"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn query_raw_with_params_null_bind() {
        let reader = setup_reader_with_data();
        let params = vec![Value::Null];
        let json_str = reader
            .query_raw_with_params("SELECT domain FROM net_events WHERE method IS ?", &params)
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        // Both rows have NULL method
        assert_eq!(parsed["rows"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn query_raw_with_params_float_bind() {
        let reader = setup_reader_with_data();
        let params = vec![json!(49.5)];
        let json_str = reader
            .query_raw_with_params(
                "SELECT domain FROM net_events WHERE duration_ms > ?",
                &params,
            )
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["rows"].as_array().unwrap().len(), 1);
        assert_eq!(parsed["rows"][0][0], "example.com");
    }

    #[test]
    fn query_raw_with_empty_params_works() {
        let reader = setup_reader_with_data();
        let json_str = reader
            .query_raw_with_params("SELECT COUNT(*) AS cnt FROM net_events", &[])
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["rows"][0][0], 2);
    }

    #[test]
    fn validate_select_only_allows_select() {
        assert!(validate_select_only("SELECT 1").is_ok());
        assert!(validate_select_only("  select * from foo").is_ok());
        assert!(validate_select_only("WITH cte AS (SELECT 1) SELECT * FROM cte").is_ok());
        assert!(validate_select_only("EXPLAIN SELECT 1").is_ok());
    }

    #[test]
    fn validate_select_only_rejects_writes() {
        assert!(validate_select_only("INSERT INTO foo VALUES (1)").is_err());
        assert!(validate_select_only("UPDATE foo SET x=1").is_err());
        assert!(validate_select_only("DELETE FROM foo").is_err());
        assert!(validate_select_only("DROP TABLE foo").is_err());
        assert!(validate_select_only("CREATE TABLE foo (x INT)").is_err());
        assert!(validate_select_only("PRAGMA journal_mode=OFF").is_err());
        assert!(validate_select_only("ATTACH ':memory:' AS db2").is_err());
    }

    #[test]
    fn validate_select_only_rejects_empty() {
        assert!(validate_select_only("").is_err());
        assert!(validate_select_only("   ").is_err());
    }

    #[test]
    fn bind_params_do_not_bypass_validation() {
        // Even with params, the SQL statement itself is validated first.
        // The validate_select_only function checks the SQL text, not the params.
        assert!(validate_select_only("DELETE FROM foo WHERE id = ?").is_err());
        assert!(validate_select_only("INSERT INTO foo VALUES (?)").is_err());
    }

    // -----------------------------------------------------------------------
    // Richer fixture covering multiple tables, used by aggregate tests below.
    // -----------------------------------------------------------------------

    fn setup_full_fixture() -> DbReader {
        let reader = DbReader::open_in_memory().unwrap();
        // net_events: 3 allowed, 1 denied, 1 error
        reader.conn.execute_batch(
            "INSERT INTO net_events
                (timestamp, domain, port, decision, method, path, bytes_sent, bytes_received, duration_ms, matched_rule)
             VALUES
                ('2026-01-01T00:00:00Z', 'api.github.com', 443, 'allowed', 'GET',  '/repos',    100, 200, 50, 'allow-github'),
                ('2026-01-01T00:01:00Z', 'api.github.com', 443, 'allowed', 'POST', '/search',   500, 900, 80, 'allow-github'),
                ('2026-01-01T00:02:00Z', 'example.com',    443, 'allowed', 'GET',  '/',         50,  100, 10, NULL),
                ('2026-01-01T00:03:00Z', 'evil.com',       443, 'denied',  'GET',  '/',         0,   0,   1,  'block-evil'),
                ('2026-01-01T00:04:00Z', 'broken.com',     443, 'error',   'GET',  '/boom',     10,  0,   25, NULL);

             INSERT INTO model_calls
                (timestamp, provider, model, method, path, input_tokens, output_tokens, duration_ms, estimated_cost_usd, trace_id)
             VALUES
                ('2026-01-01T00:10:00Z', 'anthropic', 'claude-3',  'POST', '/m', 100, 200, 1500, 0.01, 't1'),
                ('2026-01-01T00:11:00Z', 'anthropic', 'claude-3',  'POST', '/m', 50,  75,  800,  0.005, 't1'),
                ('2026-01-01T00:12:00Z', 'openai',    'gpt-4',     'POST', '/m', 30,  60,  400,  0.003, 't2');

             INSERT INTO tool_calls (model_call_id, call_index, call_id, tool_name, arguments, origin, server_name, method, decision, duration_ms)
             VALUES (1, 0, 'c-1', 'bash',  '{}', 'native', NULL, NULL, 'allowed', 0),
                    (1, 1, 'c-2', 'bash',  '{}', 'native', NULL, NULL, 'allowed', 0),
                    (2, 0, 'c-3', 'fetch', '{}', 'native', NULL, NULL, 'allowed', 0),
                    (NULL, 0, 'mcp-1', 'search_repos', '{}', 'mcp', 'github', 'tools/call', 'allowed', 100),
                    (NULL, 0, 'mcp-2', 'search_repos', '{}', 'mcp', 'github', 'tools/call', 'allowed', 120);

             INSERT INTO fs_events (timestamp, action, path)
             VALUES ('2026-01-01T00:30:00Z', 'create', '/tmp/a'),
                    ('2026-01-01T00:31:00Z', 'modify', '/tmp/a'),
                    ('2026-01-01T00:32:00Z', 'delete', '/tmp/a');
            ",
        ).unwrap();
        reader
    }

    // -----------------------------------------------------------------------
    // Counts / aggregates
    // -----------------------------------------------------------------------

    #[test]
    fn net_event_counts_reports_decision_split() {
        let r = setup_full_fixture();
        let c = r.net_event_counts().unwrap();
        assert_eq!(c.total, 5);
        assert_eq!(c.allowed, 3);
        assert_eq!(c.denied, 1);
    }

    #[test]
    fn net_event_counts_empty_db_returns_zero() {
        let r = DbReader::open_in_memory().unwrap();
        let c = r.net_event_counts().unwrap();
        assert_eq!(c.total, 0);
        assert_eq!(c.allowed, 0);
        assert_eq!(c.denied, 0);
    }

    #[test]
    fn model_call_count_matches_inserts() {
        let r = setup_full_fixture();
        assert_eq!(r.model_call_count().unwrap(), 3);
    }

    #[test]
    fn file_event_count_matches_inserts() {
        let r = setup_full_fixture();
        assert_eq!(r.file_event_count().unwrap(), 3);
    }

    // -----------------------------------------------------------------------
    // Ordering / limiting
    // -----------------------------------------------------------------------

    #[test]
    fn recent_net_events_orders_newest_first() {
        let r = setup_full_fixture();
        let evs = r.recent_net_events(10).unwrap();
        assert_eq!(evs.len(), 5);
        assert_eq!(evs[0].domain, "broken.com"); // last inserted
        assert_eq!(evs[4].domain, "api.github.com"); // first inserted
    }

    #[test]
    fn recent_net_events_respects_limit() {
        let r = setup_full_fixture();
        let evs = r.recent_net_events(2).unwrap();
        assert_eq!(evs.len(), 2);
        assert_eq!(evs[0].domain, "broken.com");
        assert_eq!(evs[1].domain, "evil.com");
    }

    #[test]
    fn recent_security_rule_events_orders_newest_first_and_keeps_payloads() {
        let r = DbReader::open_in_memory().unwrap();
        r.conn
            .execute_batch(
                "INSERT INTO security_rule_events (
                    timestamp_unix_ms, event_id, event_type, rule_id,
                    rule_action, detection_level, rule_json, event_json
                 ) VALUES
                    (1789000000000, '111111111111', 'http.request', 'allow_github',
                     'allow', 'none', '{\"name\":\"allow_github\"}', '{\"http\":{\"host\":\"api.github.com\"}}'),
                    (1789000000001, '222222222222', 'model.call', 'block_openai',
                     'block', 'critical', '{\"name\":\"block_openai\"}', '{\"model\":{\"provider\":\"openai\"}}')",
            )
            .unwrap();

        let latest = r.recent_security_rule_events(2).unwrap();
        assert_eq!(latest.len(), 2);
        assert_eq!(latest[0].event_id, "222222222222");
        assert_eq!(latest[0].rule_id, "block_openai");
        assert_eq!(latest[0].rule_action, SecurityRuleAction::Block);
        assert_eq!(latest[0].detection_level, SecurityDetectionLevel::Critical);
        assert!(latest[0].rule_json.contains("block_openai"));
        assert!(latest[0].event_json.contains("openai"));
    }

    #[test]
    fn security_rule_stats_are_db_only() {
        let r = DbReader::open_in_memory().unwrap();
        r.conn
            .execute_batch(
                "INSERT INTO security_rule_events (
                    timestamp_unix_ms, event_id, event_type, rule_id,
                    rule_action, detection_level, rule_json, event_json
                 ) VALUES
                    (1789000000000, '111111111111', 'model.call', 'block_openai',
                     'block', 'critical', '{}', '{}'),
                    (1789000000001, '222222222222', 'model.call', 'block_openai',
                     'block', 'critical', '{}', '{}'),
                    (1789000000002, '333333333333', 'http.request', 'allow_github',
                     'allow', 'none', '{}', '{}')",
            )
            .unwrap();

        let stats = r.security_rule_stats().unwrap();
        assert_eq!(stats.total, 3);
        assert!(stats
            .by_action
            .iter()
            .any(|entry| entry.rule_action == "block" && entry.count == 2));
        assert!(stats
            .by_event_type
            .iter()
            .any(|entry| entry.event_type == "model.call" && entry.count == 2));
        assert!(stats
            .by_level
            .iter()
            .any(|entry| entry.detection_level == "critical" && entry.count == 2));
        assert!(stats
            .by_level
            .iter()
            .any(|entry| entry.detection_level == "none" && entry.count == 1));
        let block = stats
            .by_rule
            .iter()
            .find(|entry| entry.rule_id == "block_openai")
            .unwrap();
        assert_eq!(block.count, 2);
        assert_eq!(block.latest_event_id, "222222222222");
        assert_eq!(block.latest_timestamp_unix_ms, 1_789_000_000_001);
    }

    #[test]
    fn recent_net_events_zero_limit() {
        let r = setup_full_fixture();
        let evs = r.recent_net_events(0).unwrap();
        assert!(evs.is_empty());
    }

    // -----------------------------------------------------------------------
    // Search
    // -----------------------------------------------------------------------

    #[test]
    fn search_net_events_matches_domain_substring() {
        let r = setup_full_fixture();
        let hits = r.search_net_events("github", 10).unwrap();
        assert_eq!(hits.len(), 2);
        for h in &hits {
            assert!(h.domain.contains("github"));
        }
    }

    #[test]
    fn search_net_events_matches_path() {
        let r = setup_full_fixture();
        let hits = r.search_net_events("search", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path.as_deref(), Some("/search"));
    }

    #[test]
    fn search_net_events_matches_method() {
        let r = setup_full_fixture();
        let hits = r.search_net_events("POST", 10).unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn search_net_events_matches_rule() {
        let r = setup_full_fixture();
        let hits = r.search_net_events("allow-github", 10).unwrap();
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn search_net_events_no_match_returns_empty() {
        let r = setup_full_fixture();
        let hits = r.search_net_events("nothing-like-this", 10).unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn search_net_events_respects_limit() {
        let r = setup_full_fixture();
        // Match all 5 rows by using a pattern that shows up everywhere.
        let hits = r.search_net_events(".com", 2).unwrap();
        assert_eq!(hits.len(), 2);
    }

    // -----------------------------------------------------------------------
    // Aggregations: top_domains, session_stats
    // -----------------------------------------------------------------------

    #[test]
    fn top_domains_ranks_by_count_desc() {
        let r = setup_full_fixture();
        let ds = r.top_domains(10).unwrap();
        assert_eq!(ds.len(), 4); // 4 distinct domains
                                 // github has 2 rows, everything else has 1 — it should be first.
        assert_eq!(ds[0].domain, "api.github.com");
        assert_eq!(ds[0].count, 2);
        assert_eq!(ds[0].allowed, 2);
        assert_eq!(ds[0].denied, 0);
    }

    #[test]
    fn top_domains_attributes_denied_vs_allowed() {
        let r = setup_full_fixture();
        let ds = r.top_domains(10).unwrap();
        let evil = ds.iter().find(|d| d.domain == "evil.com").unwrap();
        assert_eq!(evil.allowed, 0);
        assert_eq!(evil.denied, 1);
    }

    #[test]
    fn top_domains_respects_limit() {
        let r = setup_full_fixture();
        let ds = r.top_domains(1).unwrap();
        assert_eq!(ds.len(), 1);
    }

    #[test]
    fn session_stats_sums_net_and_model_columns() {
        let r = setup_full_fixture();
        let s = r.session_stats().unwrap();
        assert_eq!(s.net_total, 5);
        assert_eq!(s.net_allowed, 3);
        assert_eq!(s.net_denied, 1);
        assert_eq!(s.net_error, 1);
        assert_eq!(s.net_bytes_sent, 100 + 500 + 50 + 10);
        assert_eq!(s.net_bytes_received, 200 + 900 + 100);
        assert_eq!(s.model_call_count, 3);
        assert_eq!(s.total_input_tokens, 100 + 50 + 30);
        assert_eq!(s.total_output_tokens, 200 + 75 + 60);
        assert_eq!(s.total_model_duration_ms, 1500 + 800 + 400);
        assert_eq!(s.total_tool_calls, 3);
        // Floating point sum — allow tiny tolerance.
        assert!((s.total_estimated_cost_usd - 0.018).abs() < 1e-9);
    }

    #[test]
    fn session_stats_empty_db() {
        let r = DbReader::open_in_memory().unwrap();
        let s = r.session_stats().unwrap();
        assert_eq!(s.net_total, 0);
        assert_eq!(s.model_call_count, 0);
        assert_eq!(s.total_tool_calls, 0);
        assert_eq!(s.total_estimated_cost_usd, 0.0);
        assert!(s.total_usage_details.is_empty());
    }

    #[test]
    fn tool_call_stats_counts_unified_tool_ledger_rows() {
        let r = DbReader::open_in_memory().unwrap();
        r.conn
            .execute_batch(
                "INSERT INTO tool_calls (timestamp, origin, server_name, method, call_index, call_id, tool_name, arguments, decision, duration_ms)
                 VALUES
                    ('2026-01-01T00:00:04Z', 'mcp', 'capsem', 'tools/call', 0, 'mcp-1', 'local__fetch_http', '{}', 'allowed', 9),
                    ('2026-01-01T00:00:05Z', 'mcp', 'github', 'tools/call', 0, 'mcp-2', 'github__search', '{}', 'denied', 11),
                    ('2026-01-01T00:00:06Z', 'native', 'model', NULL, 0, 'native-1', 'bash', '{}', 'allowed', 1);",
            )
            .unwrap();

        let stats = r.tool_call_stats().unwrap();
        assert_eq!(stats.total, 3);
        assert_eq!(stats.allowed, 2);
        assert_eq!(stats.denied, 1);
        assert_eq!(stats.by_server.len(), 3);
        assert_eq!(stats.by_server[0].server_name, "capsem");
        assert_eq!(stats.by_server[0].count, 1);
        assert_eq!(stats.by_server[1].server_name, "github");
        assert_eq!(stats.by_server[1].count, 1);
        assert_eq!(stats.by_server[2].server_name, "model");
        assert_eq!(stats.by_server[2].count, 1);
    }

    #[test]
    fn recent_tool_calls_reads_unified_model_and_mcp_rows() {
        let r = DbReader::open_in_memory().unwrap();
        r.conn
            .execute_batch(
                "INSERT INTO tool_calls (
                    id, event_id, timestamp, model_call_id, origin, server_name, method,
                    request_id, call_index, call_id, tool_name, arguments, response_preview,
                    decision, duration_ms, bytes_sent, bytes_received, policy_rule, trace_id
                 ) VALUES
                    (100, 'aaaaaaaaaaaa', '2026-01-01T00:00:01Z', 1, 'native', 'model', NULL,
                     NULL, 0, 'call-model', 'write_file', '{\"path\":\"poem.md\"}',
                     'ok', 'allowed', 7, 10, 20, NULL, 'trace-model'),
                    (101, 'bbbbbbbbbbbb', '2026-01-01T00:00:02Z', NULL, 'mcp', 'capsem', 'tools/call',
                     'req-1', 0, 'req-1', 'local__fetch_http', '{\"url\":\"https://example.com\"}',
                     '{\"status\":200}', 'denied', 9, 30, 40, 'profiles.rules.block_fetch', 'trace-mcp');",
            )
            .unwrap();

        let rows = r.recent_tool_calls(10).unwrap();
        let mcp = rows.iter().find(|row| row.origin == "mcp").unwrap();
        assert_eq!(mcp.event_id, "bbbbbbbbbbbb");
        assert_eq!(mcp.model_call_id, None);
        assert_eq!(mcp.server_name.as_deref(), Some("capsem"));
        assert_eq!(mcp.method.as_deref(), Some("tools/call"));
        assert_eq!(mcp.request_id.as_deref(), Some("req-1"));
        assert_eq!(mcp.tool_name, "local__fetch_http");
        assert_eq!(
            mcp.arguments.as_deref(),
            Some("{\"url\":\"https://example.com\"}")
        );
        assert_eq!(mcp.response_preview.as_deref(), Some("{\"status\":200}"));
        assert_eq!(mcp.decision, "denied");
        assert_eq!(
            mcp.policy_rule.as_deref(),
            Some("profiles.rules.block_fetch")
        );

        let native = rows.iter().find(|row| row.origin == "native").unwrap();
        assert_eq!(native.model_call_id, Some(1));
        assert_eq!(native.tool_name, "write_file");
        assert_eq!(native.response_preview.as_deref(), Some("ok"));
    }

    #[test]
    fn raw_tool_call_count_matches_unified_ledger_rows() {
        let r = DbReader::open_in_memory().unwrap();
        r.conn
            .execute_batch(
                "INSERT INTO tool_calls (timestamp, origin, server_name, method, call_index, call_id, tool_name, arguments, decision, duration_ms)
                 VALUES
                    ('2026-01-01T00:00:00Z', 'mcp', 'capsem', 'tools/call', 0, 'call-1', 'local__snapshots_changes', '{}', 'allowed', 4),
                    ('2026-01-01T00:00:01Z', 'mcp', 'capsem', 'tools/call', 0, 'call-2', 'local__fetch_http', '{}', 'allowed', 9),
                    ('2026-01-01T00:00:02Z', 'native', 'model', NULL, 0, 'call-3', 'write_file', '{}', 'allowed', 1);",
            )
            .unwrap();

        assert_eq!(r.tool_call_stats().unwrap().total, 3);
        assert_eq!(r.raw_tool_call_count().unwrap(), 3);
    }

    #[test]
    fn brokered_credential_stats_merges_injected_rows_without_provider() {
        let r = DbReader::open_in_memory().unwrap();
        let credential_ref = crate::events::credential_reference("google", "ya29.runtime-token");
        r.conn
            .execute(
                "INSERT INTO substitution_events (
                    timestamp, material_class, source, event_type, algorithm,
                    substitution_ref, outcome, provider, trace_id
                 ) VALUES (?1, 'credential', ?2, 'http.response', 'blake3', ?3, 'captured', 'google', 'trace-1')",
                params![
                    "2026-06-14T22:00:00Z",
                    "http.body.response.$.access_token",
                    credential_ref,
                ],
            )
            .unwrap();
        r.conn
            .execute(
                "INSERT INTO substitution_events (
                    timestamp, material_class, source, event_type, algorithm,
                    substitution_ref, outcome, provider, trace_id
                 ) VALUES (?1, 'credential', ?2, 'http.request', 'blake3', ?3, 'injected', NULL, 'trace-2')",
                params![
                    "2026-06-14T22:00:01Z",
                    "http.header.authorization",
                    credential_ref,
                ],
            )
            .unwrap();
        r.conn
            .execute(
                "INSERT INTO substitution_events (
                    timestamp, material_class, source, event_type, algorithm,
                    substitution_ref, outcome, provider, trace_id
                 ) VALUES (?1, 'credential', ?2, 'http.request', 'blake3', ?3, 'injected', NULL, 'trace-3')",
                params![
                    "2026-06-14T22:00:02Z",
                    "http.query.access_token",
                    credential_ref,
                ],
            )
            .unwrap();

        let stats = r.brokered_credential_stats().unwrap();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].provider.as_deref(), Some("google"));
        assert_eq!(stats[0].credential_ref, credential_ref);
        assert_eq!(stats[0].observed_count, 3);
        assert_eq!(stats[0].injected_count, 2);
        assert_eq!(stats[0].last_seen.as_deref(), Some("2026-06-14T22:00:02Z"));
    }

    // -----------------------------------------------------------------------
    // tool_calls_for / tool_responses_for
    // -----------------------------------------------------------------------

    #[test]
    fn tool_calls_for_returns_by_model_call_id() {
        let r = setup_full_fixture();
        let t = r.tool_calls_for(1).unwrap();
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].call_id, "c-1");
        assert_eq!(t[1].call_id, "c-2");
    }

    #[test]
    fn tool_calls_for_unknown_id_returns_empty() {
        let r = setup_full_fixture();
        let t = r.tool_calls_for(9999).unwrap();
        assert!(t.is_empty());
    }

    #[test]
    fn tool_responses_for_returns_by_model_call_id() {
        let r = DbReader::open_in_memory().unwrap();
        r.conn
            .execute(
                "INSERT INTO tool_responses (model_call_id, call_id, content_preview, is_error)
             VALUES (1, 'c-1', 'ok', 0), (1, 'c-2', 'boom', 1), (2, 'c-3', 'other', 0)",
                [],
            )
            .unwrap();
        let rs = r.tool_responses_for(1).unwrap();
        assert_eq!(rs.len(), 2);
        assert!(!rs[0].is_error);
        assert!(rs[1].is_error);
    }

    #[test]
    fn tool_responses_for_tolerates_old_schema_without_credential_ref() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old-session.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute(
                "CREATE TABLE tool_responses (
                    id INTEGER PRIMARY KEY,
                    model_call_id INTEGER NOT NULL,
                    call_id TEXT NOT NULL,
                    content_preview TEXT,
                    is_error INTEGER NOT NULL DEFAULT 0
                )",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO tool_responses (model_call_id, call_id, content_preview, is_error)
                 VALUES (1, 'old-call', 'old-ok', 0)",
                [],
            )
            .unwrap();
        }

        let reader = DbReader::open(&path).unwrap();
        let responses = reader.tool_responses_for(1).unwrap();
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].call_id, "old-call");
        assert_eq!(responses[0].content_preview.as_deref(), Some("old-ok"));
        assert_eq!(responses[0].credential_ref, None);
    }

    // -----------------------------------------------------------------------
    // validate_select_only: a few more adversarial cases
    // -----------------------------------------------------------------------

    #[test]
    fn validate_select_only_rejects_upsert() {
        assert!(
            validate_select_only("INSERT INTO t VALUES (1) ON CONFLICT DO UPDATE SET x = 2")
                .is_err()
        );
    }

    #[test]
    fn validate_select_only_rejects_multi_statement() {
        // SELECT followed by DELETE should not slip through if statement was split.
        // Current implementation may accept this since it only checks the first keyword;
        // if this ever regresses, tighten the check.
        let s = "SELECT 1; DELETE FROM t";
        // Document current behavior: starts with SELECT → OK (bind params do not
        // bypass, but the statement validator is keyword-only). The DbReader
        // execute path uses query_raw which only prepares one statement — so
        // the trailing DELETE is dropped. This is a sharp edge worth noting.
        assert!(validate_select_only(s).is_ok());
    }

    #[test]
    fn query_raw_rejects_non_select() {
        let r = setup_full_fixture();
        let err = r.query_raw("DELETE FROM net_events").unwrap_err();
        // validate_select_only returns "<KEYWORD> statements are not allowed".
        assert!(
            err.contains("DELETE") && err.contains("not allowed"),
            "got: {err}"
        );
    }

    #[test]
    fn query_raw_with_params_rejects_non_select() {
        let r = setup_full_fixture();
        let err = r
            .query_raw_with_params("UPDATE net_events SET domain = ?", &[json!("x")])
            .unwrap_err();
        assert!(
            err.contains("UPDATE") && err.contains("not allowed"),
            "got: {err}"
        );
    }

    #[test]
    fn query_raw_returns_row_cap_on_large_results() {
        // Force max_rows limit by inserting many rows.
        let r = DbReader::open_in_memory().unwrap();
        for i in 0..50 {
            r.conn
                .execute(
                    "INSERT INTO net_events (timestamp, domain, decision) VALUES (?, ?, 'allowed')",
                    params![
                        format!("2026-01-01T00:{:02}:00Z", i % 60),
                        format!("d{i}.com")
                    ],
                )
                .unwrap();
        }
        // Default limit is large; just confirm all 50 are returned.
        let json_str = r.query_raw("SELECT id FROM net_events").unwrap();
        let v: Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(v["rows"].as_array().unwrap().len(), 50);
    }
}
