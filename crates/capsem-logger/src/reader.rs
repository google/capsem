use std::cell::Cell;
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

/// Aggregate security rule statistics, built from the ledger counter snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityRuleStats {
    pub total: u64,
    pub by_action: Vec<SecurityRuleActionCount>,
    pub by_event_type: Vec<SecurityRuleEventTypeCount>,
    pub by_level: Vec<SecurityRuleDetectionLevelCount>,
    pub by_rule: Vec<SecurityRuleStatsByRule>,
}

/// Shared SQL column tail for model_calls SELECT queries after provider/protocol.
const MODEL_CALL_COLUMNS_TAIL: &str = "model, process_name, pid,
     method, path, stream,
     system_prompt_preview, messages_count, tools_count,
     request_bytes, request_body_preview,
     message_id, status_code, text_content, thinking_content,
     stop_reason, input_tokens, output_tokens,
     duration_ms, response_bytes, estimated_cost_usd, trace_id";

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

    // -----------------------------------------------------------------
    // History: exec_events + audit_events
    // -----------------------------------------------------------------

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
use rows::{
    read_audit_history_row, read_exec_history_row, read_file_event_row, read_security_ask_event_row,
    read_security_rule_event_row,
};

#[cfg(test)]
mod tests;
