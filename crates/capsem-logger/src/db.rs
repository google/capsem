use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

use capsem_telemetry::db::{
    DB_QUERY_DURATION_MS, DB_QUERY_PARAMS_COUNT, DB_QUERY_RESULT_BYTES, DB_QUERY_RESULT_ROWS, DB_QUERY_TOTAL,
};

use crate::reader::DbReader;
use crate::writer::{DbWriter, WriteOp};

/// Public DB-boundary contract for Capsem session ledgers.
///
/// Callers own query intent: a stats, timeline, or security route may choose
/// the SQL projection it needs. The DB handle owns execution and storage:
/// connection threads, write queues, schema checks, WAL/mem/disk mechanics,
/// batching, flushing, and future FTS5/search tables all stay
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
/// Recently answered `query_many` batches, newest first.
///
/// One slot was enough while a single route used the batch rail. It stopped
/// being enough the moment two polled routes shared a session handle -- and
/// they do share one, exactly one per session -- because each poll evicted the
/// other's answer and neither ever hit: the handle paid the bookkeeping of a
/// cache and got the behaviour of none.
///
/// A `Vec` rather than a map because a batch's bound parameters are
/// `serde_json::Value`, which is neither `Hash` nor `Ord`. At this size,
/// comparing the batch outright is cheaper than the fingerprint that would let
/// us avoid it, and it cannot collide.
type DbQueryManyCache = Vec<(Vec<DbQueryOwned>, Vec<DbQueryJson>)>;

/// How many distinct batches one handle keeps answers for.
///
/// Two polled routes read a session handle -- `stats/summary` and
/// `security/status` -- so four leaves room for one more without anyone
/// having to come back here. The bound is on memory as much as on lookups:
/// an entry holds a whole JSON result set. Nothing in here outlives the
/// ledger state it was read from; the whole cache is dropped when the read
/// epoch moves.
const QUERY_MANY_CACHE_ENTRIES: usize = 4;

/// Typed invalidation domains for DB-owned data consumed by cached readers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadCacheDomain {
    /// Any accepted write can affect this reader.
    All,
    /// Aggregates sourced from the main DB session/usage tables. Profile
    /// mutation ledger rows are orthogonal and must not evict this projection.
    SessionSummary,
}

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
        .and_then(|value| value.get("rows").and_then(|rows| rows.as_array()).map(Vec::len))
}

fn record_query_metrics(phase: &'static str, started: Instant, params_count: usize, result: &DbResult<String>) {
    let status = if result.is_ok() { "ok" } else { "error" };
    let elapsed_ms = elapsed_ms_f64(started);
    ::metrics::counter!(DB_QUERY_TOTAL, "phase" => phase, "status" => status).increment(1);
    ::metrics::histogram!(DB_QUERY_DURATION_MS, "phase" => phase, "status" => status).record(elapsed_ms);
    ::metrics::histogram!(DB_QUERY_PARAMS_COUNT, "phase" => phase, "status" => status).record(params_count as f64);
    if let Ok(raw) = result {
        ::metrics::histogram!(DB_QUERY_RESULT_BYTES, "phase" => phase).record(raw.len() as f64);
        if let Some(rows) = query_result_rows(raw) {
            ::metrics::histogram!(DB_QUERY_RESULT_ROWS, "phase" => phase).record(rows as f64);
        }
    }
}

/// A worker reply, with whether the ledger moved under it.
///
/// `changed` is the handle's cue to expire its read caches and move the epochs
/// route caches are keyed on. It is set when the ledger file's `data_version`
/// moved: a commit by another process, or by this handle's own writer.
struct Observed<T> {
    changed: bool,
    value: T,
}

/// What the reader worker did for one `QueryMany` request.
enum QueryManyReply {
    Executed {
        changed: bool,
        results: Vec<DbQueryJson>,
    },
    /// Nothing had changed and the caller's cached result still stands, so the
    /// worker executed nothing.
    CacheStillValid,
}

/// Counters the reader worker keeps, read back by tests over the same channel
/// the queries take so no test needs a second connection to the ledger.
#[cfg(test)]
pub(crate) struct ReaderIntrospection {
    pub(crate) attached_schemas: Vec<String>,
    pub(crate) disk_syncs: u64,
    pub(crate) queries_executed: u64,
    pub(crate) busy_timeout_ms: i64,
}

enum ReadRequest {
    Ready {
        /// Carries whether the ledger changed since the worker last looked, so
        /// the handle can expire read caches keyed on its epochs.
        reply: tokio::sync::oneshot::Sender<DbResult<bool>>,
    },
    Observe {
        /// Carries whether the ledger changed since the worker last looked.
        /// Cached readiness still needs this probe before a caller can trust
        /// response bytes keyed on the read epoch.
        reply: tokio::sync::oneshot::Sender<DbResult<bool>>,
    },
    Query {
        sql: String,
        params: Vec<serde_json::Value>,
        reply: tokio::sync::oneshot::Sender<DbResult<Observed<String>>>,
    },
    QueryMany {
        queries: Vec<DbQueryOwned>,
        /// The handle holds a cached result for exactly these queries, so the
        /// worker may skip execution when the ledger did not change.
        cache_valid: bool,
        reply: tokio::sync::oneshot::Sender<DbResult<QueryManyReply>>,
    },
    CaptureBodies {
        queries: Vec<DbQueryOwned>,
        requested_ids: usize,
        reply: tokio::sync::oneshot::Sender<DbResult<Observed<bodies::CapturedBodies>>>,
    },
    CaptureWarc {
        reply: tokio::sync::oneshot::Sender<DbResult<Observed<warc_export::CapturedWarc>>>,
    },
    #[cfg(test)]
    Introspect {
        reply: tokio::sync::oneshot::Sender<DbResult<ReaderIntrospection>>,
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
/// disk-backed, memory-backed, batched, or eventually indexed for
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
    archive_blocks_inflated: AtomicU64,
    query_many_cache: Mutex<DbQueryManyCache>,
    read_cache_epoch: AtomicU64,
    session_summary_cache_epoch: AtomicU64,
    /// Parks the next `query_many` right after its cache lookup, so a test can
    /// land an invalidation exactly there. See `pause_next_query_many_for_tests`.
    #[cfg(test)]
    query_many_pause: Mutex<Option<QueryManyPause>>,
}

/// Signals "the lookup is done" and waits for "go on".
#[cfg(test)]
type QueryManyPause = (tokio::sync::oneshot::Sender<()>, tokio::sync::oneshot::Receiver<()>);

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
        // Reads go to the file, like every other handle's: the writer's
        // memory holds only rows it has not flushed, and SQLite's
        // `data_version` tells the reader when a flush landed.
        DbReader::open(path)?;
        let handle = Self::open_with_writer(path.to_path_buf(), writer)?;

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
    /// telemetry/security writes. Disk is the process boundary and WAL already
    /// makes it readable while the writer commits, so this handle queries the
    /// file directly and holds no copy of it; it caches whole `query_many`
    /// batches until SQLite's `data_version` says the writer committed. It
    /// rejects `write` so caller mistakes fail loudly instead of creating a
    /// second writer rail.
    pub fn open_external_reader(path: &Path) -> rusqlite::Result<Self> {
        let started = Instant::now();
        DbReader::open(path)?;
        let handle = Self::open_reader(path.to_path_buf())?;
        tracing::debug!(
            db_path = %path.display(),
            operation = "open_external_reader",
            duration_ms = elapsed_ms(started),
            "session db external reader handle opened"
        );
        Ok(handle)
    }

    fn open_reader(db_path: PathBuf) -> rusqlite::Result<Self> {
        let (reader_tx, reader_rx) = mpsc::channel();
        let reader_path = db_path.clone();
        let reader_join = std::thread::Builder::new()
            .name("capsem-db-reader".into())
            .spawn(move || reader_loop(reader_path, reader_rx))
            .expect("failed to spawn db reader thread");

        Ok(Self {
            inner: Arc::new(DbHandleInner {
                path: db_path,
                reader_tx,
                reader_join: Mutex::new(Some(reader_join)),
                writer: None,
                ready_cache: Mutex::new(None),
                archive_blocks_inflated: AtomicU64::new(0),
                query_many_cache: Mutex::new(DbQueryManyCache::new()),
                read_cache_epoch: AtomicU64::new(0),
                session_summary_cache_epoch: AtomicU64::new(0),
                #[cfg(test)]
                query_many_pause: Mutex::new(None),
            }),
        })
    }

    fn open_with_writer(db_path: PathBuf, writer: Arc<DbWriter>) -> rusqlite::Result<Self> {
        // Owning the writer changes only whether `write` is accepted. Its
        // writes reach the file on the writer's flush, which the reader sees
        // through `data_version` like any other commit; a local write moving
        // the epoch is not the moment the rows become readable.
        let handle = Self::open_reader(db_path)?;
        let mut inner = Arc::try_unwrap(handle.inner).ok().expect("new handle is unique");
        inner.writer = Some(writer);
        Ok(Self { inner: Arc::new(inner) })
    }

    #[cfg(test)]
    pub(crate) fn open_existing_for_tests(path: &Path) -> rusqlite::Result<Self> {
        DbReader::open(path)?;
        let writer = Arc::new(DbWriter::open_in_memory(1)?);
        Self::open_with_writer(path.to_path_buf(), writer)
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
        let cached = { self.inner.ready_cache.lock().unwrap().clone() };
        if let Some(cached) = cached {
            let changed = self.observe_reader_change().await?;
            if changed {
                self.invalidate_read_cache();
            }
            tracing::debug!(
                db_path = %self.inner.path.display(),
                operation = "ready",
                cached = true,
                changed,
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
        if matches!(result, Ok(true)) {
            // The first look at an externally written ledger, and any commit
            // since the last one, expire whatever this handle had cached.
            self.invalidate_read_cache();
        }
        let result = result.map(|_changed| ());
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

    async fn observe_reader_change(&self) -> DbResult<bool> {
        let (reply, rx) = tokio::sync::oneshot::channel();
        self.inner
            .reader_tx
            .send(ReadRequest::Observe { reply })
            .map_err(|error| format!("db reader worker closed: {error}"))?;
        rx.await
            .map_err(|error| format!("db reader worker dropped observe reply: {error}"))?
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
            .map_err(|error| format!("db reader worker dropped query reply: {error}"))?
            .map(|observed| self.take_observed(observed));
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
        // The epoch the result will belong to, read under the same lock as the
        // lookup and the invalidations, so it is exactly the epoch the cached
        // entry was valid in. Read any later and an invalidation landing in
        // between is invisible: a pre-commit answer would be stored under the
        // post-commit epoch and served as current until the next commit.
        let (mut epoch_before, cached) = self.cached_query_many(&queries);
        #[cfg(test)]
        self.pause_query_many_for_tests().await;
        // No handle can know the file is unchanged without asking SQLite --
        // not even one that owns the writer, whose flush lands on its own
        // schedule -- so a cached batch still pays one worker round trip. The
        // worker checks `data_version` and re-executes only when it moved.
        let cache_valid = cached.is_some();
        let cache_key = queries.clone();
        let (reply, rx) = tokio::sync::oneshot::channel();
        self.inner
            .reader_tx
            .send(ReadRequest::QueryMany {
                queries,
                cache_valid,
                reply,
            })
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
        let reply = rx
            .await
            .map_err(|error| format!("db reader worker dropped query_many reply: {error}"))?;
        let result = reply.map(|reply| match reply {
            QueryManyReply::CacheStillValid => cached,
            QueryManyReply::Executed { changed, results } => {
                if changed {
                    // The worker observed a commit by the other process and
                    // then executed against it: these results belong to the
                    // new epoch, not the one this call started in -- the one
                    // this invalidation created, not whatever the counter
                    // reads a moment later, which a newer commit may already
                    // have moved past these results.
                    epoch_before = self.expire_read_caches(true);
                }
                Some(results)
            }
        });
        // The worker answers `CacheStillValid` only to a request that said it
        // had one; a `None` here would be the two sides disagreeing about that,
        // which is a broken contract rather than an empty result.
        let result = result.and_then(|served| {
            served.ok_or_else(|| "db reader worker skipped execution without a cached result".to_string())
        });
        if let Ok(raw) = &result {
            self.store_query_many_cache(epoch_before, cache_key, raw.clone());
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

    /// The session's counter snapshot, as its writer last committed it.
    ///
    /// One primary-key lookup, down the same cached batch rail as every other
    /// polled read: an idle poll is answered without touching the file.
    pub async fn ledger_counters(&self) -> DbResult<crate::counters::LedgerCounters> {
        let raw = self
            .query_many(vec![(crate::counters::SNAPSHOT_SQL.to_string(), Vec::new())])
            .await?;
        let [snapshot] = raw.as_slice() else {
            return Err(format!("ledger counters returned {} results, expected 1", raw.len()));
        };
        crate::counters::from_snapshot_result(snapshot)
    }

    /// Unwrap a worker reply, expiring this handle's read caches first when the
    /// worker saw the other process commit.
    ///
    /// Every read path goes through here, so no route can be answered from a
    /// cache the ledger has already moved past, whichever entrypoint it used.
    fn take_observed<T>(&self, observed: Observed<T>) -> T {
        if observed.changed {
            self.invalidate_read_cache();
        }
        observed.value
    }

    #[cfg(test)]
    async fn introspect_reader(&self) -> DbResult<ReaderIntrospection> {
        let (reply, rx) = tokio::sync::oneshot::channel();
        self.inner
            .reader_tx
            .send(ReadRequest::Introspect { reply })
            .map_err(|error| format!("db reader worker closed: {error}"))?;
        rx.await
            .map_err(|error| format!("db reader worker dropped introspect reply: {error}"))?
    }

    /// Schema names attached to the reader worker's connection.
    #[cfg(test)]
    pub(crate) async fn attached_schemas_for_tests(&self) -> DbResult<Vec<String>> {
        Ok(self.introspect_reader().await?.attached_schemas)
    }

    /// How many times the reader worker observed the ledger change.
    #[cfg(test)]
    pub(crate) async fn disk_syncs_for_tests(&self) -> DbResult<u64> {
        Ok(self.introspect_reader().await?.disk_syncs)
    }

    /// How many caller-owned queries the reader worker actually executed;
    /// a batch served from this handle's cache never reaches it.
    #[cfg(test)]
    pub(crate) async fn queries_executed_for_tests(&self) -> DbResult<u64> {
        Ok(self.introspect_reader().await?.queries_executed)
    }

    /// How long the reader worker waits out a file lock before failing, in ms.
    #[cfg(test)]
    pub(crate) async fn busy_timeout_ms_for_tests(&self) -> DbResult<i64> {
        Ok(self.introspect_reader().await?.busy_timeout_ms)
    }

    /// The cached answer to exactly this batch, if this handle holds one.
    ///
    /// A hit moves its entry to the front, so the batches a route actually
    /// polls keep each other alive and a one-off batch is what falls off the
    /// end.
    ///
    /// Returns the read epoch alongside, taken under the cache lock. Every
    /// invalidation bumps the epoch under that same lock, so the pair is one
    /// consistent observation: the entry, if any, was valid in exactly this
    /// epoch, and a result stored against it is refused once anything expires.
    fn cached_query_many(&self, queries: &[DbQueryOwned]) -> (u64, Option<Vec<DbQueryJson>>) {
        let mut cache = self.inner.query_many_cache.lock().unwrap_or_else(|e| e.into_inner());
        let epoch = self.read_cache_epoch(ReadCacheDomain::All);
        let Some(position) = cache.iter().position(|(key, _)| key == queries) else {
            return (epoch, None);
        };
        cache[..=position].rotate_right(1);
        let result = cache[0].1.clone();
        drop(cache);
        (epoch, Some(result))
    }

    /// Cache a `query_many` result unless the read epoch moved while the
    /// query ran, in which case the result may predate a write and is dropped.
    pub(crate) fn store_query_many_cache(&self, epoch_before: u64, key: Vec<DbQueryOwned>, result: Vec<DbQueryJson>) {
        let mut cache = self.inner.query_many_cache.lock().unwrap_or_else(|e| e.into_inner());
        if self.read_cache_epoch(ReadCacheDomain::All) != epoch_before {
            tracing::debug!(
                db_path = %self.inner.path.display(),
                operation = "query_many",
                "query result predates a write; not cached"
            );
            return;
        }
        cache.retain(|(existing, _)| existing != &key);
        cache.insert(0, (key, result));
        cache.truncate(QUERY_MANY_CACHE_ENTRIES);
    }

    /// Invalidate DB-owned read caches after external logger lifecycle helpers
    /// mutate the same database.
    pub fn invalidate_read_cache(&self) {
        self.expire_read_caches(true);
    }

    fn invalidate_after_write(&self, affects_session_summary: bool) {
        self.expire_read_caches(affects_session_summary);
    }

    /// Bump the read epochs and drop every cached batch, as one step under the
    /// cache lock, and return the read epoch this created.
    ///
    /// One step because a store checks the epoch under that lock too. Clearing
    /// first and bumping after, outside it, left a window in which a result
    /// read before the commit was stored under the old epoch after the clear,
    /// and survived it.
    fn expire_read_caches(&self, affects_session_summary: bool) -> u64 {
        let mut cache = self.inner.query_many_cache.lock().unwrap_or_else(|e| e.into_inner());
        let epoch = self.inner.read_cache_epoch.fetch_add(1, Ordering::AcqRel) + 1;
        if affects_session_summary {
            self.inner.session_summary_cache_epoch.fetch_add(1, Ordering::AcqRel);
        }
        cache.clear();
        drop(cache);
        epoch
    }

    /// Monotonic generation for one typed DB read domain.
    pub fn read_cache_epoch(&self, domain: ReadCacheDomain) -> u64 {
        match domain {
            ReadCacheDomain::All => self.inner.read_cache_epoch.load(Ordering::Acquire),
            ReadCacheDomain::SessionSummary => self.inner.session_summary_cache_epoch.load(Ordering::Acquire),
        }
    }

    /// Write one telemetry/security event through the DB-owned writer path.
    ///
    /// This is the public write boundary for ledger events. The DB layer owns
    /// queuing, batching, flushing, durability mechanics, and structured
    /// operation logging. Callers must not bypass it with direct SQLite writes.
    pub async fn write(&self, op: WriteOp) -> DbResult<()> {
        let started = Instant::now();
        let op_kind = op.kind();
        let affects_session_summary = !matches!(&op, WriteOp::ProfileMutationEvent(_));
        let Some(writer) = &self.inner.writer else {
            let error = "db handle is read-only; session writes must use the owning process DB handle".to_string();
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
        self.invalidate_after_write(affects_session_summary);
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
        writer.flush_checked().await?;
        self.invalidate_read_cache();
        Ok(())
    }

    /// Park the next `query_many` on this handle just after its cache lookup.
    ///
    /// The first receiver resolves once it is parked; sending on the returned
    /// sender lets it go on. One-shot: later calls run straight through.
    #[cfg(test)]
    pub(crate) fn pause_next_query_many_for_tests(
        &self,
    ) -> (tokio::sync::oneshot::Receiver<()>, tokio::sync::oneshot::Sender<()>) {
        let (parked_tx, parked_rx) = tokio::sync::oneshot::channel();
        let (resume_tx, resume_rx) = tokio::sync::oneshot::channel();
        *self.inner.query_many_pause.lock().unwrap() = Some((parked_tx, resume_rx));
        (parked_rx, resume_tx)
    }

    #[cfg(test)]
    async fn pause_query_many_for_tests(&self) {
        let pause = self.inner.query_many_pause.lock().unwrap().take();
        if let Some((parked, resume)) = pause {
            let _ = parked.send(());
            let _ = resume.await;
        }
    }

    #[cfg(test)]
    pub(crate) async fn flush_for_tests(&self) {
        let _ = self.flush().await;
    }

    /// Raw body bytes the writer thread has staged but not yet written.
    #[cfg(test)]
    pub(crate) async fn pending_body_bytes_for_tests(&self) -> u64 {
        self.inner
            .writer
            .as_ref()
            .map_or(0, |writer| writer.pending_body_bytes())
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

mod bodies;
mod maintenance;
mod reader_worker;
mod warc_export;

pub use bodies::{ArchivedBodies, BodyDirection, StoredBody};
pub use maintenance::snapshot_session_ledger;
use reader_worker::reader_loop;
pub use warc_export::{ExportSummary, SkipReason, SkippedBody};

#[cfg(test)]
mod cache_tests;

#[cfg(test)]
mod handle_tests;

#[cfg(test)]
mod tests;
