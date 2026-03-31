use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};
use tracing::warn;

use crate::events::{FileEvent, McpCall, ModelCall, NetEvent, SnapshotEvent};
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
    SnapshotEvent(SnapshotEvent),
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

    // All senders dropped -- checkpoint WAL before closing connection.
    let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)");
}

fn execute_batch(conn: &Connection, batch: &[WriteOp]) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    for op in batch {
        match op {
            WriteOp::NetEvent(e) => insert_net_event(&tx, e)?,
            WriteOp::ModelCall(m) => insert_model_call(&tx, m)?,
            WriteOp::McpCall(c) => insert_mcp_call(&tx, c)?,
            WriteOp::FileEvent(f) => insert_file_event(&tx, f)?,
            WriteOp::SnapshotEvent(s) => insert_snapshot_event(&tx, s)?,
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

fn insert_mcp_call(conn: &Connection, call: &McpCall) -> rusqlite::Result<()> {
    let timestamp = humantime::format_rfc3339(call.timestamp).to_string();
    let req_preview = cap_field(&call.request_preview);
    let resp_preview = cap_field(&call.response_preview);
    conn.execute(
        "INSERT INTO mcp_calls (
            timestamp, server_name, method, tool_name, request_id,
            request_preview, response_preview, decision,
            duration_ms, error_message, process_name,
            bytes_sent, bytes_received
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
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
            call.bytes_sent as i64,
            call.bytes_received as i64,
        ],
    )?;
    Ok(())
}

fn insert_snapshot_event(conn: &Connection, event: &SnapshotEvent) -> rusqlite::Result<()> {
    let timestamp = humantime::format_rfc3339(event.timestamp).to_string();
    conn.execute(
        "INSERT INTO snapshot_events (
            timestamp, slot, origin, name, files_count,
            start_fs_event_id, stop_fs_event_id
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            timestamp,
            event.slot as i64,
            event.origin,
            event.name,
            event.files_count as i64,
            event.start_fs_event_id,
            event.stop_fs_event_id,
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

    #[test]
    fn db_writer_checkpoints_wal_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");

        // Write some events, then drop the writer.
        {
            let writer = DbWriter::open(&db_path, 64).unwrap();
            let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
            rt.block_on(async {
                writer
                    .write(WriteOp::FileEvent(crate::events::FileEvent {
                        timestamp: std::time::SystemTime::now(),
                        action: crate::events::FileAction::Created,
                        path: "/tmp/test".to_string(),
                        size: Some(42),
                    }))
                    .await;
            });
            // DbWriter::drop runs here -- should checkpoint WAL.
        }

        // After drop, WAL should be truncated (empty or zero-length).
        let wal_path = dir.path().join("test.db-wal");
        if wal_path.exists() {
            let wal_size = std::fs::metadata(&wal_path).unwrap().len();
            assert_eq!(wal_size, 0, "WAL should be empty after checkpoint");
        }

        // Verify data is in the main DB file (not just WAL).
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM fs_events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn snapshot_event_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("snap.db");

        {
            let writer = DbWriter::open(&db_path, 64).unwrap();
            let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
            rt.block_on(async {
                writer
                    .write(WriteOp::SnapshotEvent(crate::events::SnapshotEvent {
                        timestamp: std::time::SystemTime::UNIX_EPOCH
                            + std::time::Duration::from_secs(1_700_000_000),
                        slot: 3,
                        origin: "auto".to_string(),
                        name: None,
                        files_count: 42,
                        start_fs_event_id: 10,
                        stop_fs_event_id: 25,
                    }))
                    .await;
                writer
                    .write(WriteOp::SnapshotEvent(crate::events::SnapshotEvent {
                        timestamp: std::time::SystemTime::UNIX_EPOCH
                            + std::time::Duration::from_secs(1_700_000_100),
                        slot: 10,
                        origin: "manual".to_string(),
                        name: Some("checkpoint_1".to_string()),
                        files_count: 55,
                        start_fs_event_id: 25,
                        stop_fs_event_id: 40,
                    }))
                    .await;
            });
        }

        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM snapshot_events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 2);

        let (slot, origin, name, files, start_id, stop_id): (i64, String, Option<String>, i64, i64, i64) = conn
            .query_row(
                "SELECT slot, origin, name, files_count, start_fs_event_id, stop_fs_event_id
                 FROM snapshot_events ORDER BY id ASC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
            )
            .unwrap();
        assert_eq!(slot, 3);
        assert_eq!(origin, "auto");
        assert!(name.is_none());
        assert_eq!(files, 42);
        assert_eq!(start_id, 10);
        assert_eq!(stop_id, 25);

        let (slot2, origin2, name2): (i64, String, Option<String>) = conn
            .query_row(
                "SELECT slot, origin, name FROM snapshot_events ORDER BY id DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(slot2, 10);
        assert_eq!(origin2, "manual");
        assert_eq!(name2.as_deref(), Some("checkpoint_1"));
    }

    #[test]
    fn snapshot_fs_events_cross_reference() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("cross.db");

        {
            let writer = DbWriter::open(&db_path, 64).unwrap();
            let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
            rt.block_on(async {
                // Write some fs_events first.
                for i in 0..5 {
                    writer
                        .write(WriteOp::FileEvent(crate::events::FileEvent {
                            timestamp: std::time::SystemTime::now(),
                            action: crate::events::FileAction::Created,
                            path: format!("file_{i}.txt"),
                            size: Some(100),
                        }))
                        .await;
                }
                for i in 5..8 {
                    writer
                        .write(WriteOp::FileEvent(crate::events::FileEvent {
                            timestamp: std::time::SystemTime::now(),
                            action: crate::events::FileAction::Modified,
                            path: format!("file_{i}.txt"),
                            size: Some(200),
                        }))
                        .await;
                }
                writer
                    .write(WriteOp::FileEvent(crate::events::FileEvent {
                        timestamp: std::time::SystemTime::now(),
                        action: crate::events::FileAction::Deleted,
                        path: "old.txt".to_string(),
                        size: None,
                    }))
                    .await;

                // Snapshot 1: covers fs_events 1..5 (5 created)
                writer
                    .write(WriteOp::SnapshotEvent(crate::events::SnapshotEvent {
                        timestamp: std::time::SystemTime::now(),
                        slot: 0,
                        origin: "auto".to_string(),
                        name: None,
                        files_count: 5,
                        start_fs_event_id: 0,
                        stop_fs_event_id: 5,
                    }))
                    .await;

                // Snapshot 2: covers fs_events 6..9 (3 modified + 1 deleted)
                writer
                    .write(WriteOp::SnapshotEvent(crate::events::SnapshotEvent {
                        timestamp: std::time::SystemTime::now(),
                        slot: 1,
                        origin: "auto".to_string(),
                        name: None,
                        files_count: 8,
                        start_fs_event_id: 5,
                        stop_fs_event_id: 9,
                    }))
                    .await;
            });
        }

        let conn = rusqlite::Connection::open(&db_path).unwrap();

        // Verify snapshot 1 sees 5 created files.
        let (created, modified, deleted): (i64, i64, i64) = conn
            .query_row(
                "SELECT
                    SUM(CASE WHEN action='created' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN action='modified' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN action='deleted' THEN 1 ELSE 0 END)
                 FROM fs_events WHERE id > 0 AND id <= 5",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(created, 5);
        assert_eq!(modified, 0);
        assert_eq!(deleted, 0);

        // Verify snapshot 2 sees 3 modified + 1 deleted.
        let (created2, modified2, deleted2): (i64, i64, i64) = conn
            .query_row(
                "SELECT
                    SUM(CASE WHEN action='created' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN action='modified' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN action='deleted' THEN 1 ELSE 0 END)
                 FROM fs_events WHERE id > 5 AND id <= 9",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(created2, 0);
        assert_eq!(modified2, 3);
        assert_eq!(deleted2, 1);
    }

    #[test]
    fn snapshot_ring_buffer_dedup_query() {
        // Tests the SQL pattern used by the frontend: MAX(id) GROUP BY slot
        // ensures only the latest event per slot is returned when the ring
        // buffer overwrites a slot.
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("ring.db");

        {
            let writer = DbWriter::open(&db_path, 64).unwrap();
            let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
            rt.block_on(async {
                // Slot 0, first pass.
                writer
                    .write(WriteOp::SnapshotEvent(crate::events::SnapshotEvent {
                        timestamp: std::time::SystemTime::UNIX_EPOCH
                            + std::time::Duration::from_secs(1000),
                        slot: 0,
                        origin: "auto".to_string(),
                        name: None,
                        files_count: 5,
                        start_fs_event_id: 0,
                        stop_fs_event_id: 3,
                    }))
                    .await;
                // Slot 1.
                writer
                    .write(WriteOp::SnapshotEvent(crate::events::SnapshotEvent {
                        timestamp: std::time::SystemTime::UNIX_EPOCH
                            + std::time::Duration::from_secs(2000),
                        slot: 1,
                        origin: "auto".to_string(),
                        name: None,
                        files_count: 8,
                        start_fs_event_id: 3,
                        stop_fs_event_id: 7,
                    }))
                    .await;
                // Slot 0 again (ring buffer wrapped).
                writer
                    .write(WriteOp::SnapshotEvent(crate::events::SnapshotEvent {
                        timestamp: std::time::SystemTime::UNIX_EPOCH
                            + std::time::Duration::from_secs(3000),
                        slot: 0,
                        origin: "auto".to_string(),
                        name: None,
                        files_count: 12,
                        start_fs_event_id: 7,
                        stop_fs_event_id: 15,
                    }))
                    .await;
            });
        }

        let conn = rusqlite::Connection::open(&db_path).unwrap();

        // Total rows = 3 (all insertions).
        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM snapshot_events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(total, 3);

        // Dedup query: latest per slot. Should return 2 rows (slot 0 latest + slot 1).
        let dedup: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM snapshot_events
                 WHERE id IN (SELECT MAX(id) FROM snapshot_events GROUP BY slot)",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(dedup, 2);

        // Slot 0 should show files_count=12 (the newer entry), not 5.
        let files: i64 = conn
            .query_row(
                "SELECT files_count FROM snapshot_events
                 WHERE id IN (SELECT MAX(id) FROM snapshot_events GROUP BY slot)
                 AND slot = 0",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(files, 12);
    }
}

