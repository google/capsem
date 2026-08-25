use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

use crate::reader::DbReader;
use crate::reader::SessionStats;
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
type DbQueryOwned = (String, Vec<serde_json::Value>);
type DbQueryManyCache = Option<(Vec<DbQueryOwned>, Vec<DbQueryJson>)>;

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
    QueryMany {
        queries: Vec<DbQueryOwned>,
        reply: tokio::sync::oneshot::Sender<DbResult<Vec<String>>>,
    },
    SessionStats {
        reply: tokio::sync::oneshot::Sender<DbResult<SessionStats>>,
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
    query_many_cache: Mutex<DbQueryManyCache>,
    read_cache_epoch: AtomicU64,
    sync_from_disk_before_query: bool,
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
                query_many_cache: Mutex::new(None),
                read_cache_epoch: AtomicU64::new(0),
                sync_from_disk_before_query,
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
        // A writer in another process can still be completing canonical DDL
        // when an external reader first checks readiness.  Cache only success:
        // a transient partial-schema error must be retryable on the same
        // DB-owned handle, while a real broken schema still fails loudly.
        if result.is_ok() {
            *self.inner.ready_cache.lock().unwrap() = Some(Ok(()));
        }
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

    /// Execute several read-only queries through one DB-owned worker request.
    ///
    /// This is still caller-owned query intent and DB-owned execution. It exists
    /// for hot routes that need several independent projections but must not pay
    /// one worker round trip per projection.
    pub async fn query_many(&self, queries: Vec<DbQueryOwned>) -> DbResult<Vec<DbQueryJson>> {
        let started = Instant::now();
        let query_count = queries.len();
        let params_count: usize = queries.iter().map(|(_, params)| params.len()).sum();
        if !self.inner.sync_from_disk_before_query {
            if let Some((cached_queries, cached_result)) =
                self.inner.query_many_cache.lock().unwrap().clone()
            {
                if cached_queries == queries {
                    tracing::debug!(
                        db_path = %self.inner.path.display(),
                        operation = "query_many",
                        cached = true,
                        query_count,
                        params_count,
                        duration_ms = elapsed_ms(started),
                        "session db handle operation completed"
                    );
                    return Ok(cached_result);
                }
            }
        }
        let cache_key = queries.clone();
        let (reply, rx) = tokio::sync::oneshot::channel();
        self.inner
            .reader_tx
            .send(ReadRequest::QueryMany { queries, reply })
            .map_err(|error| {
                tracing::error!(
                    db_path = %self.inner.path.display(),
                    operation = "query_many",
                    query_count,
                    params_count,
                    duration_ms = elapsed_ms(started),
                    error = %error,
                    "session db handle operation failed"
                );
                format!("db reader worker closed: {error}")
            })?;
        let result = rx
            .await
            .map_err(|error| format!("db reader worker dropped query_many reply: {error}"))?;
        if !self.inner.sync_from_disk_before_query {
            if let Ok(raw) = &result {
                *self.inner.query_many_cache.lock().unwrap() = Some((cache_key, raw.clone()));
            }
        }
        match &result {
            Ok(_) => tracing::debug!(
                db_path = %self.inner.path.display(),
                operation = "query_many",
                query_count,
                params_count,
                duration_ms = elapsed_ms(started),
                "session db handle operation completed"
            ),
            Err(error) => tracing::error!(
                db_path = %self.inner.path.display(),
                operation = "query_many",
                query_count,
                params_count,
                duration_ms = elapsed_ms(started),
                error = %error,
                "session db handle operation failed"
            ),
        }
        result
    }

    /// Read the compact canonical session aggregates through the DB worker.
    pub async fn session_stats(&self) -> DbResult<SessionStats> {
        let (reply, rx) = tokio::sync::oneshot::channel();
        self.inner
            .reader_tx
            .send(ReadRequest::SessionStats { reply })
            .map_err(|error| format!("db reader worker closed: {error}"))?;
        rx.await
            .map_err(|error| format!("db reader worker dropped session stats reply: {error}"))?
    }

    /// Invalidate DB-owned read caches after external logger lifecycle helpers
    /// mutate the same database.
    pub fn invalidate_read_cache(&self) {
        *self.inner.query_many_cache.lock().unwrap() = None;
        self.inner.read_cache_epoch.fetch_add(1, Ordering::AcqRel);
    }

    /// Monotonic read-cache generation for callers that cache derived route
    /// bytes from DB-owned query results.
    pub fn read_cache_epoch(&self) -> u64 {
        self.inner.read_cache_epoch.load(Ordering::Acquire)
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
        self.invalidate_read_cache();
        tracing::debug!(
            db_path = %self.inner.path.display(),
            operation = "write",
            op_kind,
            duration_ms = elapsed_ms(started),
            "session db handle operation completed"
        );
        Ok(())
    }

    /// Flush accepted writes through the DB-owned writer path.
    ///
    /// Tests and read-after-write callers use this as the visibility barrier.
    /// Route code should not sleep or poll around ledger writes; the DB layer
    /// owns batching and the point at which accepted writes become queryable.
    pub async fn flush(&self) -> DbResult<()> {
        let Some(writer) = &self.inner.writer else {
            return Err("db handle is read-only; no writer is available to flush".to_string());
        };
        writer.flush().await;
        self.invalidate_read_cache();
        Ok(())
    }

    #[cfg(test)]
    pub(crate) async fn flush_for_tests(&self) {
        let _ = self.flush().await;
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
                let result = if sync_from_disk_before_query {
                    reader
                        .sync_from_disk()
                        .map_err(|error| error.to_string())
                        .and_then(|()| reader.ready())
                } else {
                    reader.ready()
                };
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
            ReadRequest::QueryMany { queries, reply } => {
                let started = Instant::now();
                let query_count = queries.len();
                let params_count: usize = queries.iter().map(|(_, params)| params.len()).sum();
                let result = if sync_from_disk_before_query {
                    reader
                        .sync_from_disk()
                        .map_err(|error| error.to_string())
                        .and_then(|()| execute_query_many(&reader, queries))
                } else {
                    execute_query_many(&reader, queries)
                };
                match &result {
                    Ok(_) => tracing::debug!(
                        db_path = %path.display(),
                        operation = "query_many_execute",
                        query_count,
                        params_count,
                        duration_ms = elapsed_ms(started),
                        "session db query batch completed"
                    ),
                    Err(error) => tracing::error!(
                        db_path = %path.display(),
                        operation = "query_many_execute",
                        query_count,
                        params_count,
                        duration_ms = elapsed_ms(started),
                        error = %error,
                        "session db query batch failed"
                    ),
                }
                let _ = reply.send(result);
            }
            ReadRequest::SessionStats { reply } => {
                let result = if sync_from_disk_before_query {
                    reader
                        .sync_from_disk()
                        .map_err(|error| error.to_string())
                        .and_then(|()| reader.session_stats().map_err(|error| error.to_string()))
                } else {
                    reader.session_stats().map_err(|error| error.to_string())
                };
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

fn execute_query_many(reader: &DbReader, queries: Vec<DbQueryOwned>) -> DbResult<Vec<String>> {
    let mut results = Vec::with_capacity(queries.len());
    for (sql, params) in queries {
        let started = Instant::now();
        let result = reader.query_raw_with_params(&sql, &params);
        record_query_metrics("execute_many", started, params.len(), &result);
        results.push(result?);
    }
    Ok(results)
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
mod tests;
