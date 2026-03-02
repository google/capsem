use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

use rusqlite::{params, Connection, OpenFlags, Row};
use serde::Serialize;
use serde_json::Value;

use crate::events::{Decision, FileAction, FileEvent, McpCall, ModelCall, NetEvent, ToolCallEntry, ToolResponseEntry};
use crate::schema;

/// Aggregate statistics for a session (computed from SQL queries).
#[derive(Debug, Clone, Serialize)]
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
}

/// Aggregate MCP call statistics.
#[derive(Debug, Clone, Serialize)]
pub struct McpCallStats {
    pub total: u64,
    pub allowed: u64,
    pub warned: u64,
    pub denied: u64,
    pub errored: u64,
    pub by_server: Vec<McpServerCallCount>,
}

/// Per-server MCP call counts.
#[derive(Debug, Clone, Serialize)]
pub struct McpServerCallCount {
    pub server_name: String,
    pub count: u64,
    pub denied: u64,
    pub warned: u64,
}

/// Shared SQL column list for model_calls SELECT queries.
const MODEL_CALL_COLUMNS: &str =
    "id, timestamp, provider, model, process_name, pid,
     method, path, stream,
     system_prompt_preview, messages_count, tools_count,
     request_bytes, request_body_preview,
     message_id, status_code, text_content, thinking_content,
     stop_reason, input_tokens, output_tokens,
     duration_ms, response_bytes, estimated_cost_usd, trace_id,
     usage_details";

/// Parse a model_calls row into (id, ModelCall). Column order must match MODEL_CALL_COLUMNS.
fn read_model_call_row(row: &Row<'_>) -> rusqlite::Result<(i64, ModelCall)> {
    let ts_str: String = row.get(1)?;
    let timestamp = humantime::parse_rfc3339(&ts_str).unwrap_or(SystemTime::UNIX_EPOCH);
    let id: i64 = row.get(0)?;

    Ok((id, ModelCall {
        timestamp,
        provider: row.get(2)?,
        model: row.get(3)?,
        process_name: row.get(4)?,
        pid: row.get::<_, Option<i64>>(5)?.map(|p| p as u32),
        method: row.get(6)?,
        path: row.get(7)?,
        stream: row.get::<_, i64>(8)? != 0,
        system_prompt_preview: row.get(9)?,
        messages_count: row.get::<_, i64>(10)? as usize,
        tools_count: row.get::<_, i64>(11)? as usize,
        request_bytes: row.get::<_, i64>(12)? as u64,
        request_body_preview: row.get(13)?,
        message_id: row.get(14)?,
        status_code: row.get::<_, Option<i64>>(15)?.map(|c| c as u16),
        text_content: row.get(16)?,
        thinking_content: row.get(17)?,
        stop_reason: row.get(18)?,
        input_tokens: row.get::<_, Option<i64>>(19)?.map(|t| t as u64),
        output_tokens: row.get::<_, Option<i64>>(20)?.map(|t| t as u64),
        usage_details: row.get::<_, Option<String>>(25)?
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default(),
        duration_ms: row.get::<_, i64>(21)? as u64,
        response_bytes: row.get::<_, i64>(22)? as u64,
        estimated_cost_usd: row.get::<_, f64>(23).unwrap_or(0.0),
        trace_id: row.get(24)?,
        tool_calls: Vec::new(),
        tool_responses: Vec::new(),
    }))
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
        "PRAGMA" | "INSERT" | "UPDATE" | "DELETE" | "DROP" | "ALTER" | "CREATE"
        | "ATTACH" | "DETACH" | "REPLACE" | "VACUUM" | "REINDEX" | "BEGIN"
        | "COMMIT" | "ROLLBACK" | "SAVEPOINT" | "RELEASE" => {
            Err(format!("{first} statements are not allowed"))
        }
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

    /// Execute an arbitrary read-only SQL query and return JSON.
    ///
    /// Returns `{"columns":[...],"rows":[[...], ...]}`.
    /// Caps output at 10,000 rows. Interrupts queries that run longer than
    /// 5 seconds via `sqlite3_interrupt`.
    pub fn query_raw(&self, sql: &str) -> Result<String, String> {
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

    fn query_raw_params_inner(&self, sql: &str, params: &[Value], max_rows: usize) -> Result<String, String> {
        let mut stmt = self.conn.prepare(sql).map_err(|e| e.to_string())?;

        let columns: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
        let col_count = columns.len();

        // Convert serde_json::Value params to rusqlite dynamic params.
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
        let mut stmt = self.conn.prepare(
            "SELECT timestamp, domain, port, decision, process_name, pid,
                    method, path, query, status_code,
                    bytes_sent, bytes_received, duration_ms, matched_rule,
                    request_headers, response_headers,
                    request_body_preview, response_body_preview, conn_type
             FROM net_events
             ORDER BY id DESC
             LIMIT ?1",
        )?;

        let rows = stmt.query_map(params![limit as i64], |row| {
            let ts_str: String = row.get(0)?;
            let timestamp =
                humantime::parse_rfc3339(&ts_str).unwrap_or(SystemTime::UNIX_EPOCH);
            let decision_str: String = row.get(3)?;

            Ok(NetEvent {
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
                conn_type: row.get(18)?,
            })
        })?;

        rows.collect()
    }

    /// Query the most recent N model calls, ordered newest first.
    /// Does NOT load nested tool_calls/tool_responses (use tool_calls_for).
    pub fn recent_model_calls(&self, limit: usize) -> rusqlite::Result<Vec<(i64, ModelCall)>> {
        let sql = format!(
            "SELECT {MODEL_CALL_COLUMNS} FROM model_calls ORDER BY id DESC LIMIT ?1"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            read_model_call_row(row)
        })?;
        rows.collect()
    }

    /// Count net events by decision: returns (total, allowed, denied).
    pub fn net_event_counts(&self) -> rusqlite::Result<(usize, usize, usize)> {
        self.conn.query_row(
            "SELECT
                COUNT(*),
                COALESCE(SUM(CASE WHEN decision = 'allowed' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN decision = 'denied' THEN 1 ELSE 0 END), 0)
             FROM net_events",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)? as usize,
                    row.get::<_, i64>(1)? as usize,
                    row.get::<_, i64>(2)? as usize,
                ))
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
                origin: row.get::<_, String>(4).unwrap_or_else(|_| "native".to_string()),
            })
        })?;
        rows.collect()
    }

    /// Get tool responses for a given model_call_id.
    pub fn tool_responses_for(&self, model_call_id: i64) -> rusqlite::Result<Vec<ToolResponseEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT call_id, content_preview, is_error
             FROM tool_responses WHERE model_call_id = ?1",
        )?;
        let rows = stmt.query_map(params![model_call_id], |row| {
            Ok(ToolResponseEntry {
                call_id: row.get(0)?,
                content_preview: row.get(1)?,
                is_error: row.get::<_, i64>(2)? != 0,
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
        let (model_call_count, total_input_tokens, total_output_tokens, total_model_duration_ms, total_estimated_cost_usd, usage_details_json) =
            self.conn.query_row(
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
            "SELECT COUNT(*) FROM tool_calls",
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
    pub fn net_events_over_time(&self, bucket_min: u64, count: usize) -> rusqlite::Result<Vec<TimeBucket>> {
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
        let rows = stmt.query_map(params![window_start as i64, bucket_sec as i64, offset], |row| {
            Ok((
                row.get::<_, i64>(0)? as usize,
                row.get::<_, i64>(1)? as u64,
                row.get::<_, i64>(2)? as u64,
            ))
        })?;

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
        let mut stmt = self.conn.prepare(
            "SELECT timestamp, domain, port, decision, process_name, pid,
                    method, path, query, status_code,
                    bytes_sent, bytes_received, duration_ms, matched_rule,
                    request_headers, response_headers,
                    request_body_preview, response_body_preview, conn_type
             FROM net_events
             WHERE domain LIKE ?1
                OR path LIKE ?1
                OR method LIKE ?1
                OR matched_rule LIKE ?1
             ORDER BY id DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![pattern, limit as i64], |row| {
            let ts_str: String = row.get(0)?;
            let timestamp = humantime::parse_rfc3339(&ts_str).unwrap_or(SystemTime::UNIX_EPOCH);
            let decision_str: String = row.get(3)?;
            Ok(NetEvent {
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
                conn_type: row.get(18)?,
            })
        })?;
        rows.collect()
    }

    /// Search model calls by provider or model substring.
    pub fn search_model_calls(&self, query: &str, limit: usize) -> rusqlite::Result<Vec<(i64, ModelCall)>> {
        let pattern = format!("%{query}%");
        let sql = format!(
            "SELECT {MODEL_CALL_COLUMNS}
             FROM model_calls
             WHERE provider LIKE ?1
                OR model LIKE ?1
                OR stop_reason LIKE ?1
             ORDER BY id DESC
             LIMIT ?2"
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
        let mut stmt = self.conn.prepare(
            "SELECT tool_name, COUNT(*) as cnt
             FROM tool_calls
             GROUP BY tool_name
             ORDER BY cnt DESC
             LIMIT ?1",
        )?;
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
        self.conn.query_row(
            "SELECT COUNT(*) FROM fs_events",
            [],
            |row| row.get::<_, i64>(0).map(|n| n as u64),
        )
    }

    /// Tool usage with response byte and duration stats from model_calls.
    pub fn tool_usage_with_stats(&self, limit: usize) -> rusqlite::Result<Vec<ToolUsageWithStats>> {
        let mut stmt = self.conn.prepare(
            "SELECT tc.tool_name, COUNT(*) as cnt,
                    COALESCE(SUM(mc.response_bytes), 0),
                    COALESCE(SUM(mc.duration_ms), 0)
             FROM tool_calls tc
             JOIN model_calls mc ON tc.model_call_id = mc.id
             GROUP BY tc.tool_name
             ORDER BY cnt DESC LIMIT ?1",
        )?;
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

    /// MCP tool usage grouped by tool_name with duration and response size.
    pub fn mcp_tool_usage(&self, limit: usize) -> rusqlite::Result<Vec<McpToolUsage>> {
        let mut stmt = self.conn.prepare(
            "SELECT tool_name, server_name, COUNT(*) as cnt,
                    COALESCE(SUM(LENGTH(response_preview)), 0),
                    COALESCE(SUM(duration_ms), 0)
             FROM mcp_calls
             WHERE tool_name IS NOT NULL
             GROUP BY tool_name
             ORDER BY cnt DESC LIMIT ?1",
        )?;
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

            let total_usage_details: BTreeMap<String, u64> = row.get::<_, Option<String>>(8)?
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
            "SELECT {MODEL_CALL_COLUMNS} FROM model_calls WHERE trace_id = ?1 ORDER BY id ASC"
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
                    origin: row.get::<_, String>(5).unwrap_or_else(|_| "native".to_string()),
                },
            ))
        })?;

        // Fetch all tool responses for this trace in one batch.
        let mut tool_resps_stmt = self.conn.prepare(
            "SELECT tr.model_call_id, tr.call_id, tr.content_preview, tr.is_error
             FROM tool_responses tr
             JOIN model_calls mc ON tr.model_call_id = mc.id
             WHERE mc.trace_id = ?1",
        )?;
        let all_tool_resps = tool_resps_stmt.query_map(params![trace_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                ToolResponseEntry {
                    call_id: row.get(1)?,
                    content_preview: row.get(2)?,
                    is_error: row.get::<_, i64>(3)? != 0,
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
        let mut stmt = self.conn.prepare(
            "SELECT timestamp, action, path, size
             FROM fs_events
             ORDER BY id DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], read_file_event_row)?;
        rows.collect()
    }

    /// Search file events by path substring.
    pub fn search_file_events(&self, query: &str, limit: usize) -> rusqlite::Result<Vec<FileEvent>> {
        let pattern = format!("%{query}%");
        let mut stmt = self.conn.prepare(
            "SELECT timestamp, action, path, size
             FROM fs_events
             WHERE path LIKE ?1
             ORDER BY id DESC
             LIMIT ?2",
        )?;
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
                COALESCE(SUM(CASE WHEN action = 'deleted' THEN 1 ELSE 0 END), 0)
             FROM fs_events",
            [],
            |row| {
                Ok(FileEventStats {
                    total: row.get::<_, i64>(0)? as u64,
                    created: row.get::<_, i64>(1)? as u64,
                    modified: row.get::<_, i64>(2)? as u64,
                    deleted: row.get::<_, i64>(3)? as u64,
                })
            },
        )
    }

    // ── MCP call queries ──────────────────────────────────────────────

    /// Query the most recent N MCP calls, ordered newest first.
    pub fn recent_mcp_calls(&self, limit: usize) -> rusqlite::Result<Vec<McpCall>> {
        let mut stmt = self.conn.prepare(
            "SELECT timestamp, server_name, method, tool_name, request_id,
                    request_preview, response_preview, decision,
                    duration_ms, error_message, process_name
             FROM mcp_calls
             ORDER BY id DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], read_mcp_call_row)?;
        rows.collect()
    }

    /// Search MCP calls by server_name, method, or tool_name substring.
    pub fn search_mcp_calls(&self, query: &str, limit: usize) -> rusqlite::Result<Vec<McpCall>> {
        let pattern = format!("%{query}%");
        let mut stmt = self.conn.prepare(
            "SELECT timestamp, server_name, method, tool_name, request_id,
                    request_preview, response_preview, decision,
                    duration_ms, error_message, process_name
             FROM mcp_calls
             WHERE server_name LIKE ?1
                OR method LIKE ?1
                OR tool_name LIKE ?1
             ORDER BY id DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![pattern, limit as i64], read_mcp_call_row)?;
        rows.collect()
    }

    /// Aggregate MCP call statistics. All aggregation done in SQL.
    pub fn mcp_call_stats(&self) -> rusqlite::Result<McpCallStats> {
        let (total, allowed, warned, denied, errored) = self.conn.query_row(
            "SELECT
                COUNT(*),
                COALESCE(SUM(CASE WHEN decision = 'allowed' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN decision = 'warned' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN decision = 'denied' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN decision = 'error' THEN 1 ELSE 0 END), 0)
             FROM mcp_calls",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)? as u64,
                    row.get::<_, i64>(1)? as u64,
                    row.get::<_, i64>(2)? as u64,
                    row.get::<_, i64>(3)? as u64,
                    row.get::<_, i64>(4)? as u64,
                ))
            },
        )?;

        let mut stmt = self.conn.prepare(
            "SELECT server_name,
                    COUNT(*) as cnt,
                    SUM(CASE WHEN decision = 'denied' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN decision = 'warned' THEN 1 ELSE 0 END)
             FROM mcp_calls
             GROUP BY server_name
             ORDER BY cnt DESC",
        )?;
        let by_server = stmt.query_map([], |row| {
            Ok(McpServerCallCount {
                server_name: row.get(0)?,
                count: row.get::<_, i64>(1)? as u64,
                denied: row.get::<_, i64>(2)? as u64,
                warned: row.get::<_, i64>(3)? as u64,
            })
        })?;

        Ok(McpCallStats {
            total,
            allowed,
            warned,
            denied,
            errored,
            by_server: by_server.collect::<rusqlite::Result<Vec<_>>>()?,
        })
    }
}

/// Parse an fs_events row into FileEvent. Column order must match the SELECT in queries above.
fn read_file_event_row(row: &Row<'_>) -> rusqlite::Result<FileEvent> {
    let ts_str: String = row.get(0)?;
    let timestamp = humantime::parse_rfc3339(&ts_str).unwrap_or(SystemTime::UNIX_EPOCH);
    let action_str: String = row.get(1)?;
    Ok(FileEvent {
        timestamp,
        action: FileAction::parse_str(&action_str),
        path: row.get(2)?,
        size: row.get::<_, Option<i64>>(3)?.map(|s| s as u64),
    })
}

/// Parse an mcp_calls row into McpCall. Column order must match the SELECT in queries above.
fn read_mcp_call_row(row: &Row<'_>) -> rusqlite::Result<McpCall> {
    let ts_str: String = row.get(0)?;
    let timestamp = humantime::parse_rfc3339(&ts_str).unwrap_or(SystemTime::UNIX_EPOCH);
    Ok(McpCall {
        timestamp,
        server_name: row.get(1)?,
        method: row.get(2)?,
        tool_name: row.get(3)?,
        request_id: row.get(4)?,
        request_preview: row.get(5)?,
        response_preview: row.get(6)?,
        decision: row.get(7)?,
        duration_ms: row.get::<_, i64>(8)? as u64,
        error_message: row.get(9)?,
        process_name: row.get(10)?,
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
        let json_str = reader.query_raw("SELECT domain, decision FROM net_events ORDER BY id").unwrap();
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
}
