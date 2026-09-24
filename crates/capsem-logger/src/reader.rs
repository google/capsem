use std::cell::Cell;
use std::collections::BTreeMap;
use std::path::Path;
use std::time::SystemTime;

use rusqlite::{params, Connection, OpenFlags, Row};
use serde::{Deserialize, Serialize};

use crate::events::{
    AuditEvent, Decision, ExecEvent, FileAction, FileEvent, FileKind, ModelCall, NetEvent, SecurityAskRecord,
    SecurityAskStatus, SecurityDetectionLevel, SecurityRuleAction, SecurityRuleMatch, ToolCallEntry, ToolResponseEntry,
};
use crate::schema;
mod columns;
mod file_events;

#[cfg(test)]
pub(crate) use columns::reader_select_columns;
use columns::{
    model_call_columns, AUDIT_EVENT_COLUMNS, AUDIT_HISTORY_COLUMNS, EXEC_EVENT_COLUMNS, EXEC_HISTORY_COLUMNS,
    NET_EVENT_COLUMNS, TOOL_CALL_COLUMNS, TOOL_RESPONSE_COLUMNS,
};
mod open;
mod schema_sync;

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
    pub transport: String,
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
    /// Changes to paths. Overflow markers are counted separately.
    pub total: u64,
    pub created: u64,
    pub modified: u64,
    pub deleted: u64,
    pub restored: u64,
    /// Windows in which the monitor saw more changes than it emitted at once.
    /// Non-zero means the rail is complete but its timing is coarser there.
    pub overflow_windows: u64,
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
            // The reader reconstructs the event from the ledger row it
            // reads, so the body it can offer is the display preview the
            // writer stored there. The archived body is reached by event_id
            // through the DB handle, never rebuilt into this struct.
            request_body: row.get::<_, Option<String>>(14)?.map(String::into_bytes),
            message_id: row.get(15)?,
            status_code: row.get::<_, Option<i64>>(16)?.map(|c| c as u16),
            text_content: row.get(17)?,
            thinking_content: row.get(18)?,
            response_body: None,
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
        "PRAGMA" | "INSERT" | "UPDATE" | "DELETE" | "DROP" | "ALTER" | "CREATE" | "ATTACH" | "DETACH" | "REPLACE"
        | "VACUUM" | "REINDEX" | "BEGIN" | "COMMIT" | "ROLLBACK" | "SAVEPOINT" | "RELEASE" => {
            Err(format!("{first} statements are not allowed"))
        }
        _ => Err(format!("unsupported statement type: {first}")),
    }
}

/// Query-only connection to the session database.
///
/// It reads the file through WAL and runs with SQLite `query_only`. Callers
/// never receive the connection and `DbHandle::query` still rejects non-read
/// SQL before execution.
pub struct DbReader {
    conn: Connection,
    /// `PRAGMA main.data_version` as of the last change this reader both saw
    /// and finished acting on. It moves only when another connection commits
    /// to the file, so an unchanged value means results derived from it are
    /// still current.
    synced_data_version: Cell<Option<i64>>,
    disk_syncs: Cell<u64>,
    queries_executed: Cell<u64>,
}

impl DbReader {
    pub(crate) fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Query the most recent N network events, ordered newest first.
    pub fn recent_net_events(&self, limit: usize) -> rusqlite::Result<Vec<NetEvent>> {
        let sql = format!(
            "SELECT {NET_EVENT_COLUMNS}
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
                request_body: row.get::<_, Option<String>>(16)?.map(String::into_bytes),
                response_body: row.get::<_, Option<String>>(17)?.map(String::into_bytes),
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
            model_call_columns()
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![limit as i64], read_model_call_row)?;
        rows.collect()
    }

    /// Recent rule matches retain their match-time snapshot; payloads are
    /// archive-backed by event id, never reconstructed from live rules.
    pub fn recent_security_rule_events(&self, limit: usize) -> rusqlite::Result<Vec<SecurityRuleMatch>> {
        let mut stmt = self.conn.prepare(
            "SELECT event.timestamp_unix_ms, event.event_id, event.event_type, event.rule_id,
                    event.rule_action, event.detection_level,
                    COALESCE(event.rule_json, run.rule_json), event.trace_id,
                    event.turn_id, event.credential_ref
             FROM security_rule_events AS event
             LEFT JOIN security_rule_runs AS run ON run.id = event.run_id
             ORDER BY event.timestamp_unix_ms DESC, event.id DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], read_security_rule_event_row)?;
        rows.collect()
    }

    /// Query recent ask lifecycle records, newest first.
    pub fn recent_security_ask_events(&self, limit: usize) -> rusqlite::Result<Vec<SecurityAskRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT timestamp_unix_ms, ask_id, event_id, event_type, rule_id,
                    rule_name, status, rule_json, resolver, reason, trace_id
             FROM security_ask_events
             ORDER BY timestamp_unix_ms DESC, id DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], read_security_ask_event_row)?;
        rows.collect()
    }

    /// Return the latest lifecycle row for an ask id.
    pub fn latest_security_ask_event(&self, ask_id: &str) -> rusqlite::Result<Option<SecurityAskRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT timestamp_unix_ms, ask_id, event_id, event_type, rule_id,
                    rule_name, status, rule_json, resolver, reason, trace_id
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
        let total = self
            .conn
            .query_row("SELECT COALESCE(SUM(count), 0) FROM security_rule_runs", [], |row| {
                row.get::<_, i64>(0).map(|value| value as u64)
            })?;

        let mut action_stmt = self.conn.prepare(
            "SELECT rule_action, SUM(count) FROM security_rule_runs
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
            "SELECT event_type, SUM(count) FROM security_rule_runs
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
            "SELECT detection_level, SUM(count) FROM security_rule_runs
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
                SUM(sre.count) AS count,
                (
                    SELECT latest.event_id
                    FROM security_rule_events latest
                    WHERE latest.rule_id = sre.rule_id
                      AND latest.rule_action = sre.rule_action
                      AND latest.detection_level = sre.detection_level
                    ORDER BY latest.timestamp_unix_ms DESC, latest.id DESC
                    LIMIT 1
                ) AS latest_event_id,
                MAX(sre.last_timestamp_unix_ms) AS latest_timestamp_unix_ms
             FROM security_rule_runs sre
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
        self.conn.query_row("SELECT COUNT(*) FROM model_calls", [], |row| {
            row.get::<_, i64>(0).map(|n| n as usize)
        })
    }

    /// Get tool calls for a given model_call_id.
    pub fn tool_calls_for(&self, model_call_id: i64) -> rusqlite::Result<Vec<ToolCallEntry>> {
        let sql = format!("SELECT {TOOL_CALL_COLUMNS} FROM tool_calls WHERE model_call_id = ?1 ORDER BY call_index");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![model_call_id], |row| {
            Ok(ToolCallEntry {
                event_id: row.get(5)?,
                call_index: row.get::<_, i64>(0)? as u32,
                call_id: row.get(1)?,
                tool_name: row.get(2)?,
                arguments: row.get(3)?,
                origin: row.get::<_, String>(4).unwrap_or_else(|_| "native".to_string()),
                trace_id: None,
            })
        })?;
        rows.collect()
    }

    /// Get tool responses for a given model_call_id.
    pub fn tool_responses_for(&self, model_call_id: i64) -> rusqlite::Result<Vec<ToolResponseEntry>> {
        let sql = format!("SELECT {TOOL_RESPONSE_COLUMNS} FROM tool_responses WHERE model_call_id = ?1");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![model_call_id], |row| {
            Ok(ToolResponseEntry {
                event_id: row.get(4)?,
                call_id: row.get(0)?,
                content_preview: row.get(1)?,
                is_error: row.get::<_, i64>(2)? != 0,
                trace_id: None,
                credential_ref: row.get(3)?,
            })
        })?;
        rows.collect()
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
        let sql = format!(
            "SELECT {NET_EVENT_COLUMNS}
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
                request_body: row.get::<_, Option<String>>(16)?.map(String::into_bytes),
                response_body: row.get::<_, Option<String>>(17)?.map(String::into_bytes),
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
    pub fn search_model_calls(&self, query: &str, limit: usize) -> rusqlite::Result<Vec<(i64, ModelCall)>> {
        let pattern = format!("%{query}%");
        let sql = format!(
            "SELECT {}
             FROM model_calls
             WHERE provider LIKE ?1
                OR model LIKE ?1
                OR stop_reason LIKE ?1
             ORDER BY id DESC
             LIMIT ?2",
            model_call_columns()
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![pattern, limit as i64], read_model_call_row)?;
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
        self.conn.query_row("SELECT COUNT(*) FROM fs_events", [], |row| {
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
            model_call_columns()
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows: Vec<(i64, ModelCall)> = stmt
            .query_map(params![trace_id], read_model_call_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        // Fetch all tool calls for this trace in one batch.
        let mut tool_calls_stmt = self.conn.prepare(
            "SELECT tc.model_call_id, tc.call_index, tc.call_id, tc.tool_name, tc.arguments, tc.origin, tc.event_id
             FROM tool_calls tc
             JOIN model_calls mc ON tc.model_call_id = mc.id
             WHERE mc.trace_id = ?1
             ORDER BY tc.model_call_id, tc.call_index",
        )?;
        let all_tool_calls = tool_calls_stmt.query_map(params![trace_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                ToolCallEntry {
                    event_id: row.get(6)?,
                    call_index: row.get::<_, i64>(1)? as u32,
                    call_id: row.get(2)?,
                    tool_name: row.get(3)?,
                    arguments: row.get(4)?,
                    origin: row.get::<_, String>(5).unwrap_or_else(|_| "native".to_string()),
                    trace_id: None,
                },
            ))
        })?;

        // Fetch all tool responses for this trace in one batch.
        let mut tool_resps_stmt = self.conn.prepare(
            "SELECT tr.model_call_id, tr.call_id, tr.content_preview, tr.is_error, tr.credential_ref, tr.event_id
             FROM tool_responses tr
             JOIN model_calls mc ON tr.model_call_id = mc.id
             WHERE mc.trace_id = ?1",
        )?;
        let all_tool_resps = tool_resps_stmt.query_map(params![trace_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                ToolResponseEntry {
                    event_id: row.get(5)?,
                    call_id: row.get(1)?,
                    content_preview: row.get(2)?,
                    is_error: row.get::<_, i64>(3)? != 0,
                    trace_id: None,
                    credential_ref: row.get(4)?,
                },
            ))
        })?;

        // Group by model_call_id.
        let mut tool_calls_map: std::collections::HashMap<i64, Vec<ToolCallEntry>> = std::collections::HashMap::new();
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

    /// Query the user-facing tool-call ledger, ordered newest first.
    pub fn recent_tool_calls(&self, limit: usize) -> rusqlite::Result<Vec<ToolCallLedgerEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, event_id, timestamp, model_call_id, origin, transport, server_name, method,
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
                transport: row.get(5)?,
                server_name: row.get(6)?,
                method: row.get(7)?,
                request_id: row.get(8)?,
                call_id: row.get(9)?,
                tool_name: row.get(10)?,
                arguments: row.get(11)?,
                response_preview: row.get(12)?,
                decision: row.get(13)?,
                duration_ms: row.get::<_, i64>(14)? as u64,
                error_message: row.get(15)?,
                bytes_sent: row.get::<_, i64>(16)? as u64,
                bytes_received: row.get::<_, i64>(17)? as u64,
                policy_rule: row.get(18)?,
                trace_id: row.get(19)?,
                credential_ref: row.get(20)?,
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
        let (total, allowed, warned, denied, errored) = self.conn.query_row(&totals_sql, [], |row| {
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
        self.conn.query_row("SELECT COUNT(*) FROM tool_calls", [], |row| {
            Ok(row.get::<_, i64>(0)? as u64)
        })
    }

    // -----------------------------------------------------------------
    // History: exec_events + audit_events
    // -----------------------------------------------------------------

    /// Counts of exec and audit events in this session.
    pub fn history_counts(&self) -> rusqlite::Result<HistoryCounts> {
        let exec_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM exec_events", [], |row| row.get(0))?;
        let audit_count: i64 = self
            .conn
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
                let sql = format!(
                    "SELECT {EXEC_HISTORY_COLUMNS} FROM exec_events WHERE command LIKE ?1 ORDER BY timestamp DESC"
                );
                let mut stmt = self.conn.prepare(&sql)?;
                let rows = stmt.query_map(params![pattern], read_exec_history_row)?;
                for r in rows {
                    entries.push(r?);
                }
            } else {
                let sql = format!("SELECT {EXEC_HISTORY_COLUMNS} FROM exec_events ORDER BY timestamp DESC");
                let mut stmt = self.conn.prepare(&sql)?;
                let rows = stmt.query_map([], read_exec_history_row)?;
                for r in rows {
                    entries.push(r?);
                }
            }
        }

        if layer == "all" || layer == "audit" {
            if let Some(q) = search {
                let pattern = format!("%{q}%");
                let sql = format!(
                    "SELECT {AUDIT_HISTORY_COLUMNS} FROM audit_events \
                     WHERE argv LIKE ?1 OR exe LIKE ?1 ORDER BY timestamp DESC"
                );
                let mut stmt = self.conn.prepare(&sql)?;
                let rows = stmt.query_map(params![pattern], read_audit_history_row)?;
                for r in rows {
                    entries.push(r?);
                }
            } else {
                let sql = format!("SELECT {AUDIT_HISTORY_COLUMNS} FROM audit_events ORDER BY timestamp DESC");
                let mut stmt = self.conn.prepare(&sql)?;
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
        let sql = format!("SELECT {EXEC_EVENT_COLUMNS} FROM exec_events ORDER BY timestamp DESC LIMIT ?1");
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
        let sql = format!("SELECT {AUDIT_EVENT_COLUMNS} FROM audit_events ORDER BY timestamp DESC LIMIT ?1");
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

mod rawquery;
mod rows;
pub(crate) mod session_stats;
use rows::{
    read_audit_history_row, read_exec_history_row, read_file_event_row, read_security_ask_event_row,
    read_security_rule_event_row,
};

#[cfg(test)]
mod tests;
