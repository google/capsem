use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

use crate::reader::DbReader;
use crate::writer::{DbWriter, WriteOp};

/// Public DB-boundary contract for Capsem session ledgers.
///
/// Callers own query intent: a stats, timeline, or security route may choose
/// the SQL projection it needs. The DB handle owns execution and storage:
/// connection threads, write queues, schema checks, WAL/mem/disk mechanics,
/// batching, flushing, rehydration, and future FTS5/search tables all stay
/// inside `capsem-logger`.
///
/// Required caller rail:
///
/// ```text
/// db.ready().await?;
/// db.query(sql, params).await?;
/// db.write(event).await?;
/// ```
///
/// Empty valid tables return empty results. Missing tables, missing columns,
/// non-read SQL through `query`, or closed workers are hard contract failures;
/// callers must not convert those into fake empty route responses.
pub const DB_HANDLE_CONTRACT: &str =
    "caller owns query intent; db owns execution and storage; missing schema fails loudly";

/// Result type returned by the public asynchronous DB handle API.
///
/// The error string is already contextualized by the DB layer and is logged
/// with structured fields at the boundary. Route code should add its own route
/// context when converting this to HTTP/UDS errors, not special-case schema
/// failures into empty data.
pub type DbResult<T> = Result<T, String>;

/// Bound parameter list for `DbHandle::query`.
///
/// The DB layer owns conversion into SQLite parameters. Callers pass JSON
/// scalar values only as query intent; they do not own a SQLite connection.
pub type DbQueryParams = [serde_json::Value];

/// JSON object returned by `DbHandle::query`.
///
/// The value is encoded as `{ "columns": [...], "rows": [...] }`, matching
/// `DbReader::query_raw_with_params`. Routes may map it into product JSON, but
/// execution and schema failures remain DB-owned.
pub type DbQueryJson = String;

pub const DB_QUERY_TOTAL: &str = "db.query_total";
pub const DB_QUERY_DURATION_MS: &str = "db.query_duration_ms";
pub const DB_QUERY_RESULT_ROWS: &str = "db.query_result_rows";
pub const DB_QUERY_RESULT_BYTES: &str = "db.query_result_bytes";
pub const DB_QUERY_PARAMS_COUNT: &str = "db.query_params_count";

fn elapsed_ms(started: Instant) -> u128 {
    started.elapsed().as_millis()
}

fn elapsed_ms_f64(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

fn sql_fingerprint(sql: &str) -> String {
    let hash = blake3::hash(sql.as_bytes()).to_hex();
    hash[..12].to_string()
}

fn query_result_rows(raw: &str) -> Option<usize> {
    serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|value| {
            value
                .get("rows")
                .and_then(|rows| rows.as_array())
                .map(Vec::len)
        })
}

fn record_query_metrics(
    phase: &'static str,
    started: Instant,
    params_count: usize,
    result: &DbResult<String>,
) {
    let status = if result.is_ok() { "ok" } else { "error" };
    let elapsed_ms = elapsed_ms_f64(started);
    ::metrics::counter!(DB_QUERY_TOTAL, "phase" => phase, "status" => status).increment(1);
    ::metrics::histogram!(DB_QUERY_DURATION_MS, "phase" => phase, "status" => status)
        .record(elapsed_ms);
    ::metrics::histogram!(DB_QUERY_PARAMS_COUNT, "phase" => phase, "status" => status)
        .record(params_count as f64);
    if let Ok(raw) = result {
        ::metrics::histogram!(DB_QUERY_RESULT_BYTES, "phase" => phase).record(raw.len() as f64);
        if let Some(rows) = query_result_rows(raw) {
            ::metrics::histogram!(DB_QUERY_RESULT_ROWS, "phase" => phase).record(rows as f64);
        }
    }
}

enum ReadRequest {
    Ready {
        reply: tokio::sync::oneshot::Sender<DbResult<()>>,
    },
    Query {
        sql: String,
        params: Vec<serde_json::Value>,
        reply: tokio::sync::oneshot::Sender<DbResult<String>>,
    },
    Shutdown,
}

/// Session DB path wrapper.
///
/// `SessionDb` is a construction helper for session-owned code that has a path
/// and needs the logger-owned DB objects. Product routes should prefer
/// `SessionDb::handle` or an already-open `DbHandle`; they should not construct
/// raw SQLite readers or writers themselves.
pub struct SessionDb {
    path: PathBuf,
}

/// Logger-owned handle for all session ledger DB execution.
///
/// This is the public boundary for session telemetry/security ledgers. It owns
/// the reader worker and writer queue and hides whether the implementation is
/// disk-backed, memory-backed, batched, rehydrated, or eventually indexed for
/// search. Callers may provide SQL because they own query intent; callers may
/// not own SQLite connections, route projections, missing-schema fallbacks, or
/// write buffering.
#[derive(Clone)]
pub struct DbHandle {
    inner: Arc<DbHandleInner>,
}

struct DbHandleInner {
    path: PathBuf,
    reader_tx: mpsc::Sender<ReadRequest>,
    reader_join: Mutex<Option<JoinHandle<()>>>,
    writer: Option<Arc<DbWriter>>,
    ready_cache: Mutex<Option<DbResult<()>>>,
}

impl Drop for DbHandleInner {
    fn drop(&mut self) {
        let _ = self.reader_tx.send(ReadRequest::Shutdown);
        if let Some(handle) = self.reader_join.lock().unwrap().take() {
            let _ = handle.join();
        }
    }
}

impl DbHandle {
    /// Open the session DB handle and start DB-owned workers.
    ///
    /// Opening applies the logger schema through the writer path, validates a
    /// reader can open the same DB, and starts a DB-owned reader worker. Route
    /// code receives a handle; it does not receive a connection.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let started = Instant::now();
        let writer = Arc::new(DbWriter::open(path, 1024)?);
        DbReader::open(path)?;
        let handle = Self::open_with_writer(path.to_path_buf(), writer, false)?;

        tracing::debug!(
            db_path = %path.display(),
            operation = "open",
            duration_ms = elapsed_ms(started),
            "session db handle opened"
        );

        Ok(handle)
    }

    /// Open a DB handle for a session DB written by another process.
    ///
    /// Capsem service routes read session ledgers, but capsem-process owns the
    /// telemetry/security writes. This handle keeps the same `ready/query`
    /// contract while syncing its DB-owned memory tables from disk before
    /// reads. It rejects `write` so caller mistakes fail loudly instead of
    /// creating a second writer rail.
    pub fn open_external_reader(path: &Path) -> rusqlite::Result<Self> {
        let started = Instant::now();
        DbReader::open(path)?;
        let handle = Self::open_reader(path.to_path_buf(), true)?;
        tracing::debug!(
            db_path = %path.display(),
            operation = "open_external_reader",
            duration_ms = elapsed_ms(started),
            "session db external reader handle opened"
        );
        Ok(handle)
    }

    fn open_reader(db_path: PathBuf, sync_from_disk_before_query: bool) -> rusqlite::Result<Self> {
        let (reader_tx, reader_rx) = mpsc::channel();
        let reader_path = db_path.clone();
        let reader_join = std::thread::Builder::new()
            .name("capsem-db-reader".into())
            .spawn(move || reader_loop(reader_path, reader_rx, sync_from_disk_before_query))
            .expect("failed to spawn db reader thread");

        Ok(Self {
            inner: Arc::new(DbHandleInner {
                path: db_path,
                reader_tx,
                reader_join: Mutex::new(Some(reader_join)),
                writer: None,
                ready_cache: Mutex::new(None),
            }),
        })
    }

    fn open_with_writer(
        db_path: PathBuf,
        writer: Arc<DbWriter>,
        sync_from_disk_before_query: bool,
    ) -> rusqlite::Result<Self> {
        let handle = Self::open_reader(db_path, sync_from_disk_before_query)?;
        let mut inner = Arc::try_unwrap(handle.inner)
            .ok()
            .expect("new handle is unique");
        inner.writer = Some(writer);
        Ok(Self {
            inner: Arc::new(inner),
        })
    }

    #[cfg(test)]
    pub(crate) fn open_existing_for_tests(path: &Path) -> rusqlite::Result<Self> {
        DbReader::open(path)?;
        let writer = Arc::new(DbWriter::open_in_memory(1)?);
        Self::open_with_writer(path.to_path_buf(), writer, false)
    }

    pub fn path(&self) -> &Path {
        &self.inner.path
    }

    /// Verify the DB handle is usable before a route depends on it.
    ///
    /// This is the readiness contract entrypoint for routes. The contract is
    /// intentionally stable: as the DB layer grows schema/migration/mem-table
    /// checks, callers keep invoking `ready().await` and do not learn about the
    /// internal storage strategy.
    pub async fn ready(&self) -> DbResult<()> {
        let started = Instant::now();
        if let Some(cached) = self.inner.ready_cache.lock().unwrap().clone() {
            tracing::debug!(
                db_path = %self.inner.path.display(),
                operation = "ready",
                cached = true,
                duration_ms = elapsed_ms(started),
                "session db handle operation completed"
            );
            return cached;
        }
        let (reply, rx) = tokio::sync::oneshot::channel();
        self.inner
            .reader_tx
            .send(ReadRequest::Ready { reply })
            .map_err(|error| {
                tracing::error!(
                    db_path = %self.inner.path.display(),
                    operation = "ready",
                    duration_ms = elapsed_ms(started),
                    error = %error,
                    "session db handle operation failed"
                );
                format!("db reader worker closed: {error}")
            })?;
        let result = rx
            .await
            .map_err(|error| format!("db reader worker dropped ready reply: {error}"))?;
        match &result {
            Ok(()) => tracing::debug!(
                db_path = %self.inner.path.display(),
                operation = "ready",
                duration_ms = elapsed_ms(started),
                "session db handle operation completed"
            ),
            Err(error) => tracing::error!(
                db_path = %self.inner.path.display(),
                operation = "ready",
                duration_ms = elapsed_ms(started),
                error = %error,
                "session db handle operation failed"
            ),
        }
        *self.inner.ready_cache.lock().unwrap() = Some(result.clone());
        result
    }

    /// Execute one read-only query through the DB-owned worker.
    ///
    /// `sql` is caller-owned query intent. Execution, parameter binding,
    /// connection ownership, structured logging, and schema failure semantics
    /// are owned by the DB layer. Non-read SQL and broken schema fail loudly.
    pub async fn query(&self, sql: &str, params: &DbQueryParams) -> DbResult<DbQueryJson> {
        let started = Instant::now();
        let sql_hash = sql_fingerprint(sql);
        let params_count = params.len();
        let (reply, rx) = tokio::sync::oneshot::channel();
        self.inner
            .reader_tx
            .send(ReadRequest::Query {
                sql: sql.to_string(),
                params: params.to_vec(),
                reply,
            })
            .map_err(|error| {
                tracing::error!(
                    db_path = %self.inner.path.display(),
                    operation = "query",
                    sql_hash,
                    params_count,
                    duration_ms = elapsed_ms(started),
                    error = %error,
                    "session db handle operation failed"
                );
                format!("db reader worker closed: {error}")
            })?;
        let result = rx
            .await
            .map_err(|error| format!("db reader worker dropped query reply: {error}"))?;
        record_query_metrics("handle", started, params_count, &result);
        match &result {
            Ok(_) => tracing::debug!(
                db_path = %self.inner.path.display(),
                operation = "query",
                sql_hash,
                params_count,
                duration_ms = elapsed_ms(started),
                "session db handle operation completed"
            ),
            Err(error) => tracing::error!(
                db_path = %self.inner.path.display(),
                operation = "query",
                sql_hash,
                params_count,
                duration_ms = elapsed_ms(started),
                error = %error,
                "session db handle operation failed"
            ),
        }
        result
    }

    /// Write one telemetry/security event through the DB-owned writer path.
    ///
    /// This is the public write boundary for ledger events. The DB layer owns
    /// queuing, batching, flushing, durability mechanics, and structured
    /// operation logging. Callers must not bypass it with direct SQLite writes.
    pub async fn write(&self, op: WriteOp) -> DbResult<()> {
        let started = Instant::now();
        let op_kind = op.kind();
        let Some(writer) = &self.inner.writer else {
            let error =
                "db handle is read-only; session writes must use the owning process DB handle"
                    .to_string();
            tracing::error!(
                db_path = %self.inner.path.display(),
                operation = "write",
                op_kind,
                duration_ms = elapsed_ms(started),
                error = %error,
                "session db handle operation failed"
            );
            return Err(error);
        };
        writer.write_checked(op).await.map_err(|error| {
            tracing::error!(
                db_path = %self.inner.path.display(),
                operation = "write",
                op_kind,
                duration_ms = elapsed_ms(started),
                error = %error,
                "session db handle operation failed"
            );
            error
        })?;
        tracing::debug!(
            db_path = %self.inner.path.display(),
            operation = "write",
            op_kind,
            duration_ms = elapsed_ms(started),
            "session db handle operation completed"
        );
        Ok(())
    }

    #[cfg(test)]
    pub(crate) async fn flush_for_tests(&self) {
        if let Some(writer) = &self.inner.writer {
            writer.flush().await;
        }
    }

    /// Transitional blocking readiness bridge for legacy synchronous callers.
    ///
    /// New async route code should use `ready().await`. This method exists only
    /// while service routes are being moved behind persistent async DB handles.
    pub fn ready_blocking(&self) -> rusqlite::Result<()> {
        match DbReader::open(&self.inner.path).and_then(|reader| {
            reader
                .ready()
                .map_err(rusqlite::Error::InvalidParameterName)
        }) {
            Ok(()) => Ok(()),
            Err(error) => {
                tracing::error!(
                    db_path = %self.inner.path.display(),
                    operation = "ready_blocking",
                    error = %error,
                    "session db operation failed"
                );
                Err(error)
            }
        }
    }

    /// Transitional blocking query bridge for legacy synchronous callers.
    ///
    /// New async route code should use `query(sql, params).await`. This method
    /// must not grow route-specific behavior or missing-schema compatibility.
    pub fn query_raw_blocking(&self, sql: &str) -> Result<String, String> {
        self.with_reader_string(|reader| reader.query_raw(sql).map_err(|error| error.to_string()))
    }

    /// Transitional blocking reader bridge for legacy typed reader methods.
    ///
    /// New route work should flow through `query`; future sprint items burn
    /// this bridge as handles move into service session state.
    pub fn with_reader_blocking<T>(
        &self,
        f: impl FnOnce(&DbReader) -> rusqlite::Result<T>,
    ) -> rusqlite::Result<T> {
        let reader = match DbReader::open(&self.inner.path) {
            Ok(reader) => reader,
            Err(error) => {
                tracing::error!(
                    db_path = %self.inner.path.display(),
                    operation = "open_reader_blocking",
                    error = %error,
                    "session db operation failed"
                );
                return Err(error);
            }
        };
        f(&reader)
    }

    fn with_reader_string<T>(
        &self,
        f: impl FnOnce(&DbReader) -> Result<T, String>,
    ) -> Result<T, String> {
        let reader = DbReader::open(&self.inner.path).map_err(|error| {
            tracing::error!(
                db_path = %self.inner.path.display(),
                operation = "open_reader_blocking",
                error = %error,
                "session db operation failed"
            );
            error.to_string()
        })?;
        f(&reader)
    }
}

fn reader_loop(path: PathBuf, rx: mpsc::Receiver<ReadRequest>, sync_from_disk_before_query: bool) {
    let started = Instant::now();
    let reader = match DbReader::open(&path) {
        Ok(reader) => reader,
        Err(error) => {
            tracing::error!(
                db_path = %path.display(),
                operation = "reader_worker_open",
                error = %error,
                "session db reader worker failed"
            );
            return;
        }
    };
    tracing::debug!(
        db_path = %path.display(),
        operation = "reader_worker_open",
        duration_ms = elapsed_ms(started),
        "session db reader worker opened"
    );

    while let Ok(request) = rx.recv() {
        match request {
            ReadRequest::Ready { reply } => {
                let started = Instant::now();
                let result = reader.ready();
                match &result {
                    Ok(()) => tracing::debug!(
                        db_path = %path.display(),
                        operation = "ready_execute",
                        duration_ms = elapsed_ms(started),
                        "session db readiness completed"
                    ),
                    Err(error) => tracing::error!(
                        db_path = %path.display(),
                        operation = "ready_execute",
                        duration_ms = elapsed_ms(started),
                        error = %error,
                        "session db readiness failed"
                    ),
                }
                let _ = reply.send(result);
            }
            ReadRequest::Query { sql, params, reply } => {
                let started = Instant::now();
                let sql_hash = sql_fingerprint(&sql);
                let params_count = params.len();
                let result = if sync_from_disk_before_query {
                    reader
                        .sync_from_disk()
                        .map_err(|error| error.to_string())
                        .and_then(|()| reader.query_raw_with_params(&sql, &params))
                } else {
                    reader.query_raw_with_params(&sql, &params)
                };
                record_query_metrics("execute", started, params_count, &result);
                match &result {
                    Ok(_) => tracing::debug!(
                        db_path = %path.display(),
                        operation = "query_execute",
                        sql_hash,
                        params_count,
                        duration_ms = elapsed_ms(started),
                        "session db query completed"
                    ),
                    Err(error) => tracing::error!(
                        db_path = %path.display(),
                        operation = "query_execute",
                        sql_hash,
                        params_count,
                        duration_ms = elapsed_ms(started),
                        error = %error,
                        "session db query failed"
                    ),
                }
                let _ = reply.send(result);
            }
            ReadRequest::Shutdown => {
                tracing::debug!(
                    db_path = %path.display(),
                    operation = "reader_worker_shutdown",
                    "session db reader worker shutting down"
                );
                break;
            }
        }
    }
}

impl SessionDb {
    /// Create a new SessionDb pointing at the given path.
    /// Does not open any connections; call `writer()` or `reader()` as needed.
    pub fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
        }
    }

    /// The path to the database file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Open a writer (spawns a dedicated thread).
    pub fn writer(&self, capacity: usize) -> rusqlite::Result<DbWriter> {
        DbWriter::open(&self.path, capacity)
    }

    /// Open a read-only connection.
    pub fn reader(&self) -> rusqlite::Result<DbReader> {
        DbReader::open(&self.path)
    }

    pub fn handle(&self) -> rusqlite::Result<DbHandle> {
        DbHandle::open(&self.path)
    }
}

/// Checkpoint and vacuum a session ledger.
///
/// The logger crate owns SQLite execution. Core/session code may decide when a
/// ledger needs compaction, but the actual SQLite work stays behind this
/// boundary.
pub fn checkpoint_and_vacuum_session_db(path: &Path) -> anyhow::Result<()> {
    let conn = rusqlite::Connection::open(path).map_err(|error| {
        tracing::error!(
            db_path = %path.display(),
            operation = "checkpoint_vacuum_open",
            error = %error,
            "session db maintenance failed"
        );
        error
    })?;
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .map_err(|error| {
            tracing::error!(
                db_path = %path.display(),
                operation = "wal_checkpoint_truncate",
                error = %error,
                "session db maintenance failed"
            );
            error
        })?;
    conn.execute_batch("VACUUM").map_err(|error| {
        tracing::error!(
            db_path = %path.display(),
            operation = "vacuum",
            error = %error,
            "session db maintenance failed"
        );
        error
    })?;
    tracing::debug!(
        db_path = %path.display(),
        operation = "checkpoint_and_vacuum",
        "session db maintenance completed"
    );
    Ok(())
}

/// Clone a session ledger into a new SQLite database with `VACUUM INTO`.
///
/// This creates a coherent snapshot without exposing raw SQLite connection
/// ownership to snapshot or filesystem code.
pub fn snapshot_session_db(src: &Path, dst: &Path) -> anyhow::Result<()> {
    let src_conn = rusqlite::Connection::open_with_flags(
        src,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| {
        tracing::error!(
            src_db_path = %src.display(),
            dst_db_path = %dst.display(),
            operation = "snapshot_open_source",
            error = %error,
            "session db snapshot failed"
        );
        error
    })?;

    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            tracing::error!(
                src_db_path = %src.display(),
                dst_db_path = %dst.display(),
                parent_path = %parent.display(),
                operation = "snapshot_create_parent",
                error = %error,
                "session db snapshot failed"
            );
            error
        })?;
    }
    let _ = std::fs::remove_file(dst);
    let escaped = dst.to_string_lossy().replace('\'', "''");
    src_conn
        .execute_batch(&format!("VACUUM INTO '{escaped}';"))
        .map_err(|error| {
            tracing::error!(
                src_db_path = %src.display(),
                dst_db_path = %dst.display(),
                operation = "snapshot_vacuum_into",
                error = %error,
                "session db snapshot failed"
            );
            error
        })?;
    drop(src_conn);

    let dst_conn = rusqlite::Connection::open_with_flags(
        dst,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| {
        tracing::error!(
            src_db_path = %src.display(),
            dst_db_path = %dst.display(),
            operation = "snapshot_open_destination",
            error = %error,
            "session db snapshot failed"
        );
        error
    })?;
    let quick_check: String = dst_conn
        .pragma_query_value(None, "quick_check", |row| row.get(0))
        .map_err(|error| {
            tracing::error!(
                src_db_path = %src.display(),
                dst_db_path = %dst.display(),
                operation = "snapshot_quick_check",
                error = %error,
                "session db snapshot failed"
            );
            error
        })?;
    if quick_check.eq_ignore_ascii_case("ok") {
        tracing::debug!(
            src_db_path = %src.display(),
            dst_db_path = %dst.display(),
            operation = "snapshot",
            "session db snapshot completed"
        );
        Ok(())
    } else {
        tracing::error!(
            src_db_path = %src.display(),
            dst_db_path = %dst.display(),
            operation = "snapshot_quick_check",
            quick_check,
            "session db snapshot failed"
        );
        anyhow::bail!("cloned session db failed quick_check: {quick_check}")
    }
}

#[cfg(test)]
mod handle_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{Decision, McpCall, ModelCall, NetEvent, ToolCallEntry, ToolResponseEntry};
    use std::time::SystemTime;

    fn temp_db_path(name: &str) -> PathBuf {
        let p =
            std::env::temp_dir().join(format!("capsem-test-db-{name}-{}.db", std::process::id()));
        // Clean up any stale file/WAL from previous runs
        let _ = std::fs::remove_file(&p);
        let _ = std::fs::remove_file(p.with_extension("db-wal"));
        let _ = std::fs::remove_file(p.with_extension("db-shm"));
        p
    }

    fn make_net_event(domain: &str, decision: Decision) -> NetEvent {
        NetEvent {
            event_id: None,
            timestamp: SystemTime::now(),
            domain: domain.to_string(),
            port: 443,
            decision,
            process_name: Some("test".into()),
            pid: Some(1),
            method: Some("GET".into()),
            path: Some("/api".into()),
            query: None,
            status_code: Some(200),
            bytes_sent: 100,
            bytes_received: 500,
            duration_ms: 50,
            matched_rule: None,
            request_headers: None,
            response_headers: None,
            request_body_preview: None,
            response_body_preview: None,
            request_body_full: None,
            response_body_full: None,
            conn_type: None,
            policy_mode: None,
            policy_action: None,
            policy_rule: None,
            policy_reason: None,
            trace_id: None,
            credential_ref: None,
        }
    }

    fn make_model_call() -> ModelCall {
        ModelCall {
            event_id: None,
            timestamp: SystemTime::now(),
            provider: "anthropic".into(),
            protocol: Some("anthropic".into()),
            model: Some("claude-sonnet-4-20250514".into()),
            process_name: Some("claude".into()),
            pid: Some(42),
            method: "POST".into(),
            path: "/v1/messages".into(),
            stream: true,
            system_prompt_preview: None,
            messages_count: 3,
            tools_count: 1,
            request_bytes: 1024,
            request_body_preview: None,
            request_body_full: None,
            message_id: Some("msg_123".into()),
            status_code: Some(200),
            text_content: Some("Hello".into()),
            thinking_content: None,
            response_body_full: None,
            stop_reason: Some("end_turn".into()),
            input_tokens: Some(100),
            output_tokens: Some(50),
            usage_details: Default::default(),
            duration_ms: 1200,
            response_bytes: 2048,
            estimated_cost_usd: 0.003,
            trace_id: Some("trace_abc".into()),
            credential_ref: None,
            tool_calls: vec![ToolCallEntry {
                call_index: 0,
                call_id: "call_001".into(),
                tool_name: "write_file".into(),
                arguments: Some(r#"{"path":"test.txt"}"#.into()),
                origin: "native".into(),
                trace_id: None,
            }],
            tool_responses: vec![ToolResponseEntry {
                call_id: "call_001".into(),
                content_preview: Some("ok".into()),
                is_error: false,
                trace_id: None,
                credential_ref: None,
            }],
        }
    }

    #[test]
    fn session_db_path() {
        let db = SessionDb::new(Path::new("/tmp/test.db"));
        assert_eq!(db.path(), Path::new("/tmp/test.db"));
    }

    #[test]
    fn writer_creates_tables() {
        let p = temp_db_path("creates-tables");
        let _writer = DbWriter::open(&p, 16).expect("open writer");
        drop(_writer);

        // Verify tables exist by opening a reader and querying
        let reader = DbReader::open(&p).expect("open reader");
        let counts = reader.net_event_counts().unwrap();
        assert_eq!(counts.total, 0);

        std::fs::remove_file(&p).ok();
    }

    #[tokio::test]
    async fn write_read_roundtrip_net_event() {
        let p = temp_db_path("rt-net");
        let writer = DbWriter::open(&p, 16).unwrap();
        let event = make_net_event("example.com", Decision::Allowed);

        writer.write(crate::WriteOp::NetEvent(event)).await;
        drop(writer); // flush

        let reader = DbReader::open(&p).unwrap();
        let events = reader.recent_net_events(10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].domain, "example.com");
        assert_eq!(events[0].decision, Decision::Allowed);

        std::fs::remove_file(&p).ok();
    }

    #[tokio::test]
    async fn write_read_roundtrip_model_call() {
        let p = temp_db_path("rt-model");
        let writer = DbWriter::open(&p, 16).unwrap();
        let mc = make_model_call();

        writer.write(crate::WriteOp::ModelCall(mc)).await;
        drop(writer);

        let reader = DbReader::open(&p).unwrap();
        let calls = reader.recent_model_calls(10).unwrap();
        assert_eq!(calls.len(), 1);
        let (id, call) = &calls[0];
        assert_eq!(call.provider, "anthropic");
        assert_eq!(call.trace_id.as_deref(), Some("trace_abc"));

        let tools = reader.tool_calls_for(*id).unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].tool_name, "write_file");
        assert_eq!(tools[0].origin, "native");

        let resps = reader.tool_responses_for(*id).unwrap();
        assert_eq!(resps.len(), 1);
        assert_eq!(resps[0].call_id, "call_001");
        assert!(!resps[0].is_error);

        std::fs::remove_file(&p).ok();
    }

    #[tokio::test]
    async fn write_read_roundtrip_mcp_call() {
        let p = temp_db_path("rt-mcp");
        let writer = DbWriter::open(&p, 16).unwrap();

        let mcp = McpCall {
            event_id: None,
            timestamp: SystemTime::now(),
            server_name: "builtin".into(),
            method: "tools/call".into(),
            tool_name: Some("fetch_http".into()),
            request_id: Some("req_1".into()),
            request_preview: Some("{}".into()),
            response_preview: Some("{\"ok\":true}".into()),
            decision: "allowed".into(),
            duration_ms: 100,
            error_message: None,
            process_name: Some("claude".into()),
            bytes_sent: 50,
            bytes_received: 200,
            transport: "vsock_frame".into(),
            policy_mode: None,
            policy_action: None,
            policy_rule: None,
            policy_reason: None,
            trace_id: None,
            credential_ref: None,
        };
        writer.write(crate::WriteOp::McpCall(mcp)).await;
        drop(writer);

        let conn = rusqlite::Connection::open(&p).unwrap();
        let (origin, server, method, tool, decision): (String, String, String, String, String) =
            conn.query_row(
                "SELECT origin, server_name, method, tool_name, decision FROM tool_calls",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(origin, "mcp");
        assert_eq!(server, "builtin");
        assert_eq!(method, "tools/call");
        assert_eq!(tool, "fetch_http");
        assert_eq!(decision, "allowed");

        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn empty_db_returns_zero_counts() {
        let p = temp_db_path("empty-counts");
        let writer = DbWriter::open(&p, 16).unwrap();
        drop(writer);

        let reader = DbReader::open(&p).unwrap();
        let counts = reader.net_event_counts().unwrap();
        assert_eq!(counts.total, 0);
        assert_eq!(counts.allowed, 0);
        assert_eq!(reader.model_call_count().unwrap(), 0);
        assert_eq!(reader.file_event_count().unwrap(), 0);

        let stats = reader.session_stats().unwrap();
        assert_eq!(stats.net_total, 0);
        assert_eq!(stats.model_call_count, 0);

        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn reader_on_nonexistent_path_fails() {
        let result = DbReader::open(Path::new("/nonexistent/db.sqlite"));
        assert!(result.is_err());
    }

    #[test]
    fn writer_creates_wal_mode() {
        let p = temp_db_path("wal-mode");
        let _writer = DbWriter::open(&p, 16).unwrap();
        drop(_writer);

        // WAL files should have been created (or checkpoint cleared them)
        let conn = rusqlite::Connection::open(&p).unwrap();
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");

        std::fs::remove_file(&p).ok();
    }

    #[tokio::test]
    async fn concurrent_writes_dont_corrupt() {
        let p = temp_db_path("concurrent");
        let writer = DbWriter::open(&p, 64).unwrap();

        for i in 0..50 {
            let domain = format!("domain-{}.com", i);
            writer
                .write(crate::WriteOp::NetEvent(make_net_event(
                    &domain,
                    if i % 2 == 0 {
                        Decision::Allowed
                    } else {
                        Decision::Denied
                    },
                )))
                .await;
        }
        writer.flush().await;
        drop(writer);

        let reader = DbReader::open(&p).unwrap();
        let counts = reader.net_event_counts().unwrap();
        assert_eq!(counts.total, 50);
        assert_eq!(counts.allowed, 25);
        assert_eq!(counts.denied, 25);

        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn query_raw_returns_json() {
        let p = temp_db_path("query-raw");
        let _writer = DbWriter::open(&p, 16).unwrap();
        drop(_writer);

        let reader = DbReader::open(&p).unwrap();
        let json = reader
            .query_raw("SELECT COUNT(*) as cnt FROM net_events")
            .unwrap();
        assert!(json.contains("cnt"));
        assert!(json.contains("0"));

        std::fs::remove_file(&p).ok();
    }

    #[tokio::test]
    async fn wal_survives_close_reopen() {
        let p = temp_db_path("wal-reopen");

        let writer = DbWriter::open(&p, 16).unwrap();
        writer
            .write(crate::WriteOp::NetEvent(make_net_event(
                "a.com",
                Decision::Allowed,
            )))
            .await;
        writer
            .write(crate::WriteOp::NetEvent(make_net_event(
                "b.com",
                Decision::Denied,
            )))
            .await;
        drop(writer);

        let reader = DbReader::open(&p).unwrap();
        let c = reader.net_event_counts().unwrap();
        assert_eq!((c.total, c.allowed, c.denied), (2, 1, 1));

        let writer2 = DbWriter::open(&p, 16).unwrap();
        writer2
            .write(crate::WriteOp::NetEvent(make_net_event(
                "c.com",
                Decision::Error,
            )))
            .await;
        drop(writer2);

        let reader2 = DbReader::open(&p).unwrap();
        let c2 = reader2.net_event_counts().unwrap();
        assert_eq!((c2.total, c2.allowed, c2.denied), (3, 1, 1));

        std::fs::remove_file(&p).ok();
    }
}
