use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};
use tracing::warn;

use crate::events::{FileEvent, McpCall, ModelCall, NetEvent};
use crate::schema;

/// Maximum bytes stored for any preview/content field (256 KB).
/// Callers should truncate before constructing events, but the logger
/// enforces this defensively to prevent unbounded storage.
const MAX_FIELD_BYTES: usize = 256 * 1024;

/// Truncate an optional string field to MAX_FIELD_BYTES.
fn cap_field(s: &Option<String>) -> Option<String> {
    s.as_ref().map(|v| {
        if v.len() <= MAX_FIELD_BYTES {
            v.clone()
        } else {
            // Truncate at a char boundary to avoid invalid UTF-8.
            let mut end = MAX_FIELD_BYTES;
            while end > 0 && !v.is_char_boundary(end) {
                end -= 1;
            }
            v[..end].to_string()
        }
    })
}

/// Typed write operations sent to the writer thread.
#[derive(Debug)]
pub enum WriteOp {
    NetEvent(NetEvent),
    ModelCall(ModelCall),
    McpCall(McpCall),
    FileEvent(FileEvent),
}

/// A dedicated writer thread that owns the SQLite connection.
///
/// Callers send `WriteOp` values through an mpsc channel. The writer thread
/// blocks until ops arrive, drains the queue, and executes them in a single
/// transaction for efficiency.
pub struct DbWriter {
    /// Wrapped in Option so Drop can take+drop it BEFORE joining the thread.
    /// Without this, Drop would deadlock: join waits for the thread, but the
    /// thread waits for all Senders to drop, and self.tx drops AFTER Drop body.
    tx: Option<tokio::sync::mpsc::Sender<WriteOp>>,
    join_handle: Option<std::thread::JoinHandle<()>>,
    db_path: PathBuf,
}

impl DbWriter {
    /// Spawn a dedicated writer thread that owns the DB connection.
    /// `capacity` controls the mpsc channel size (backpressure).
    pub fn open(path: &Path, capacity: usize) -> rusqlite::Result<Self> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let conn = Connection::open(path)?;
        schema::apply_pragmas(&conn)?;
        schema::create_tables(&conn)?;
        schema::migrate(&conn);

        let (tx, rx) = tokio::sync::mpsc::channel(capacity);
        let db_path = path.to_path_buf();

        let join_handle = std::thread::Builder::new()
            .name("capsem-db-writer".into())
            .spawn(move || writer_loop(conn, rx))
            .expect("failed to spawn db writer thread");

        Ok(Self {
            tx: Some(tx),
            join_handle: Some(join_handle),
            db_path,
        })
    }

    /// Open an in-memory database (for testing).
    pub fn open_in_memory(capacity: usize) -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        schema::apply_pragmas(&conn)?;
        schema::create_tables(&conn)?;
        schema::migrate(&conn);

        let (tx, rx) = tokio::sync::mpsc::channel(capacity);

        let join_handle = std::thread::Builder::new()
            .name("capsem-db-writer".into())
            .spawn(move || writer_loop(conn, rx))
            .expect("failed to spawn db writer thread");

        Ok(Self {
            tx: Some(tx),
            join_handle: Some(join_handle),
            db_path: PathBuf::from(":memory:"),
        })
    }

    /// Non-blocking send from async context. Yields if channel full (backpressure).
    pub async fn write(&self, op: WriteOp) {
        if let Some(tx) = &self.tx {
            if let Err(e) = tx.send(op).await {
                warn!(error = %e, "db writer channel closed, dropping write op");
            }
        }
    }

    /// Try to send without blocking. Returns false if the channel is full or closed.
    pub fn try_write(&self, op: WriteOp) -> bool {
        self.tx.as_ref().is_some_and(|tx| tx.try_send(op).is_ok())
    }

    /// Open a read-only connection to the same DB file (WAL concurrent reader).
    /// Returns Err for in-memory writers (no file to share between connections).
    pub fn reader(&self) -> rusqlite::Result<crate::reader::DbReader> {
        if self.db_path.to_str() == Some(":memory:") {
            return Err(rusqlite::Error::InvalidPath(self.db_path.clone()));
        }
        crate::reader::DbReader::open(&self.db_path)
    }

    /// The path to the database file.
    pub fn path(&self) -> &Path {
        &self.db_path
    }
}

impl Drop for DbWriter {
    fn drop(&mut self) {
        // Drop tx FIRST to unblock the writer thread's blocking_recv().
        // Without this, join() below would deadlock: the thread waits for
        // all Senders to drop, but field drops happen AFTER the Drop body.
        self.tx.take();
        if let Some(handle) = self.join_handle.take() {
            let _ = handle.join();
        }
    }
}

/// The writer thread loop: block-then-drain batching.
fn writer_loop(conn: Connection, mut rx: tokio::sync::mpsc::Receiver<WriteOp>) {
    loop {
        // 1. Block until at least one op arrives.
        //    Returns None when all Senders are dropped (clean shutdown).
        let Some(first_op) = rx.blocking_recv() else {
            break;
        };

        let mut batch = Vec::with_capacity(128);
        batch.push(first_op);

        // 2. Drain any ops already queued (non-blocking).
        while let Ok(op) = rx.try_recv() {
            batch.push(op);
            if batch.len() >= 128 {
                break;
            }
        }

        // 3. Execute entire batch in a single transaction.
        if let Err(e) = execute_batch(&conn, &batch) {
            warn!(error = %e, count = batch.len(), "db write batch failed");
        }
    }
}

fn execute_batch(conn: &Connection, batch: &[WriteOp]) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    for op in batch {
        match op {
            WriteOp::NetEvent(e) => insert_net_event(&tx, e)?,
            WriteOp::ModelCall(m) => insert_model_call(&tx, m)?,
            WriteOp::McpCall(c) => insert_mcp_call(&tx, c)?,
            WriteOp::FileEvent(f) => insert_file_event(&tx, f)?,
        }
    }
    tx.commit()
}

fn insert_net_event(conn: &Connection, event: &NetEvent) -> rusqlite::Result<()> {
    let timestamp = humantime::format_rfc3339(event.timestamp).to_string();
    let req_body = cap_field(&event.request_body_preview);
    let resp_body = cap_field(&event.response_body_preview);
    let req_headers = cap_field(&event.request_headers);
    let resp_headers = cap_field(&event.response_headers);
    conn.execute(
        "INSERT INTO net_events (
            timestamp, domain, port, decision, process_name, pid,
            method, path, query, status_code,
            bytes_sent, bytes_received, duration_ms, matched_rule,
            request_headers, response_headers,
            request_body_preview, response_body_preview, conn_type
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
        params![
            timestamp,
            event.domain,
            event.port as i64,
            event.decision.as_str(),
            event.process_name,
            event.pid.map(|p| p as i64),
            event.method,
            event.path,
            event.query,
            event.status_code.map(|c| c as i64),
            event.bytes_sent as i64,
            event.bytes_received as i64,
            event.duration_ms as i64,
            event.matched_rule,
            req_headers,
            resp_headers,
            req_body,
            resp_body,
            event.conn_type,
        ],
    )?;
    Ok(())
}

fn insert_model_call(conn: &Connection, call: &ModelCall) -> rusqlite::Result<()> {
    let timestamp = humantime::format_rfc3339(call.timestamp).to_string();
    let req_body = cap_field(&call.request_body_preview);
    let text_content = cap_field(&call.text_content);
    let thinking_content = cap_field(&call.thinking_content);
    let sys_prompt = cap_field(&call.system_prompt_preview);
    conn.execute(
        "INSERT INTO model_calls (
            timestamp, provider, model, process_name, pid,
            method, path, stream,
            system_prompt_preview, messages_count, tools_count,
            request_bytes, request_body_preview,
            message_id, status_code, text_content, thinking_content,
            stop_reason, input_tokens, output_tokens,
            duration_ms, response_bytes, estimated_cost_usd, trace_id,
            usage_details
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25)",
        params![
            timestamp,
            call.provider,
            call.model,
            call.process_name,
            call.pid.map(|p| p as i64),
            call.method,
            call.path,
            call.stream as i64,
            sys_prompt,
            call.messages_count as i64,
            call.tools_count as i64,
            call.request_bytes as i64,
            req_body,
            call.message_id,
            call.status_code.map(|c| c as i64),
            text_content,
            thinking_content,
            call.stop_reason,
            call.input_tokens.map(|t| t as i64),
            call.output_tokens.map(|t| t as i64),
            call.duration_ms as i64,
            call.response_bytes as i64,
            call.estimated_cost_usd,
            call.trace_id,
            if call.usage_details.is_empty() { None } else { Some(serde_json::to_string(&call.usage_details).unwrap_or_default()) },
        ],
    )?;
    let model_call_id = conn.last_insert_rowid();

    for tc in &call.tool_calls {
        conn.execute(
            "INSERT INTO tool_calls (model_call_id, call_index, call_id, tool_name, arguments, origin)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                model_call_id,
                tc.call_index as i64,
                tc.call_id,
                tc.tool_name,
                tc.arguments,
                tc.origin,
            ],
        )?;
    }

    for tr in &call.tool_responses {
        conn.execute(
            "INSERT INTO tool_responses (model_call_id, call_id, content_preview, is_error)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                model_call_id,
                tr.call_id,
                tr.content_preview,
                tr.is_error as i64,
            ],
        )?;
    }

    Ok(())
}

fn insert_file_event(conn: &Connection, event: &FileEvent) -> rusqlite::Result<()> {
    let timestamp = humantime::format_rfc3339(event.timestamp).to_string();
    conn.execute(
        "INSERT INTO fs_events (timestamp, action, path, size)
         VALUES (?1, ?2, ?3, ?4)",
        params![
            timestamp,
            event.action.as_str(),
            event.path,
            event.size.map(|s| s as i64),
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_field_none_returns_none() {
        assert!(cap_field(&None).is_none());
    }

    #[test]
    fn cap_field_short_string_unchanged() {
        let s = Some("hello world".to_string());
        assert_eq!(cap_field(&s).as_deref(), Some("hello world"));
    }

    #[test]
    fn cap_field_exact_limit_unchanged() {
        let s = Some("x".repeat(MAX_FIELD_BYTES));
        let result = cap_field(&s).unwrap();
        assert_eq!(result.len(), MAX_FIELD_BYTES);
    }

    #[test]
    fn cap_field_over_limit_truncated() {
        let s = Some("a".repeat(MAX_FIELD_BYTES + 100));
        let result = cap_field(&s).unwrap();
        assert_eq!(result.len(), MAX_FIELD_BYTES);
    }

    #[test]
    fn cap_field_utf8_boundary_safe() {
        // Multi-byte UTF-8: each char is 4 bytes
        let emoji = "\u{1F600}"; // 4-byte emoji
        assert_eq!(emoji.len(), 4);
        // Fill up to just past the limit with 4-byte chars
        let count = MAX_FIELD_BYTES / 4 + 1; // slightly over
        let s = Some(emoji.repeat(count));
        let result = cap_field(&s).unwrap();
        assert!(result.len() <= MAX_FIELD_BYTES);
        // Truncated at a char boundary -- must be valid UTF-8
        assert!(result.is_char_boundary(result.len()));
        // Length should be a multiple of 4 (each emoji is 4 bytes)
        assert_eq!(result.len() % 4, 0);
    }

    #[test]
    fn cap_field_two_byte_utf8_boundary() {
        // 2-byte char: e.g. 'a' with accent
        let ch = "\u{00E9}"; // e-acute, 2 bytes
        assert_eq!(ch.len(), 2);
        let count = MAX_FIELD_BYTES / 2 + 1;
        let s = Some(ch.repeat(count));
        let result = cap_field(&s).unwrap();
        assert!(result.len() <= MAX_FIELD_BYTES);
        assert_eq!(result.len() % 2, 0);
    }

    #[test]
    fn cap_field_three_byte_utf8_boundary() {
        // 3-byte char: CJK character
        let ch = "\u{4E16}"; // Chinese char, 3 bytes
        assert_eq!(ch.len(), 3);
        let count = MAX_FIELD_BYTES / 3 + 1;
        let s = Some(ch.repeat(count));
        let result = cap_field(&s).unwrap();
        assert!(result.len() <= MAX_FIELD_BYTES);
        assert_eq!(result.len() % 3, 0);
    }

    #[test]
    fn cap_field_empty_string_unchanged() {
        let s = Some(String::new());
        assert_eq!(cap_field(&s).as_deref(), Some(""));
    }

    #[test]
    fn cap_field_mixed_ascii_and_multibyte() {
        // Fill most of the buffer with ASCII, end with a 4-byte char that straddles the limit
        let mut s = "x".repeat(MAX_FIELD_BYTES - 1);
        s.push('\u{1F600}'); // 4 bytes, total = MAX_FIELD_BYTES + 3
        let result = cap_field(&Some(s)).unwrap();
        assert!(result.len() <= MAX_FIELD_BYTES);
        // Should have truncated to MAX_FIELD_BYTES - 1 (dropping the emoji)
        assert_eq!(result.len(), MAX_FIELD_BYTES - 1);
        assert!(result.chars().all(|c| c == 'x'));
    }
}

fn insert_mcp_call(conn: &Connection, call: &McpCall) -> rusqlite::Result<()> {
    let timestamp = humantime::format_rfc3339(call.timestamp).to_string();
    let req_preview = cap_field(&call.request_preview);
    let resp_preview = cap_field(&call.response_preview);
    conn.execute(
        "INSERT INTO mcp_calls (
            timestamp, server_name, method, tool_name, request_id,
            request_preview, response_preview, decision,
            duration_ms, error_message, process_name
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            timestamp,
            call.server_name,
            call.method,
            call.tool_name,
            call.request_id,
            req_preview,
            resp_preview,
            call.decision,
            call.duration_ms as i64,
            call.error_message,
            call.process_name,
        ],
    )?;
    Ok(())
}
