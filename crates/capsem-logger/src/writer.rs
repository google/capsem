use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

use rusqlite::{params, Connection};
use tracing::{warn, Instrument};
use uuid::Uuid;

use crate::events::{
    AuditEvent, DnsEvent, ExecEvent, ExecEventComplete, FileEvent, McpCall, ModelCall, NetEvent,
    ProfileMutationEvent, SecurityAskEvent, SecurityDecisionEvent, SecurityRuleEvent,
    SubstitutionEvent,
};
use crate::schema;

/// Maximum bytes stored for any preview/content field (256 KB).
/// Callers should truncate before constructing events, but the logger
/// enforces this defensively to prevent unbounded storage.
const MAX_FIELD_BYTES: usize = 256 * 1024;
const MAX_BODY_BLOB_BYTES: usize = 10 * 1024 * 1024;

pub const DB_ENQUEUE_SPAN: &str = "capsem.db.enqueue";
pub const DB_WRITE_BATCH_SPAN: &str = "capsem.db.write_batch";
pub const DB_SHUTDOWN_FLUSH_SPAN: &str = "capsem.db.shutdown_flush";

pub const DB_ENQUEUE_WAIT_MS: &str = "db.enqueue_wait_ms";
pub const DB_WRITE_BATCH_TOTAL: &str = "db.write_batch_total";
pub const DB_WRITE_BATCH_DURATION_MS: &str = "db.write_batch_duration_ms";
pub const DB_WRITE_BATCH_SIZE: &str = "db.write_batch_size";
pub const DB_SHUTDOWN_FLUSH_MS: &str = "db.shutdown_flush_ms";

fn new_event_id() -> String {
    let value = Uuid::new_v4().simple().to_string();
    value[..12].to_string()
}

fn format_timestamp(timestamp: SystemTime) -> String {
    humantime::format_rfc3339_micros(timestamp).to_string()
}

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

fn blake3_ref(value: &str) -> String {
    format!("blake3:{}", blake3::hash(value.as_bytes()).to_hex())
}

fn blake3_bytes_ref(value: &[u8]) -> String {
    format!("blake3:{}", blake3::hash(value).to_hex())
}

type ModelItemDedup = HashSet<String>;

fn model_item_dedup_key(
    trace_id: Option<&str>,
    kind: &str,
    content_hash: &str,
    call_id: &str,
) -> String {
    format!(
        "{}\0{}\0{}\0{}",
        trace_id.unwrap_or_default(),
        kind,
        content_hash,
        call_id
    )
}

/// Typed write operations sent to the writer thread.
#[derive(Debug, Clone)]
pub enum WriteOp {
    NetEvent(NetEvent),
    ModelCall(ModelCall),
    McpCall(McpCall),
    FileEvent(FileEvent),
    ExecEvent(ExecEvent),
    ExecEventComplete(ExecEventComplete),
    AuditEvent(AuditEvent),
    DnsEvent(DnsEvent),
    SubstitutionEvent(SubstitutionEvent),
    SecurityRuleEvent(SecurityRuleEvent),
    SecurityAskEvent(SecurityAskEvent),
    SecurityDecisionEvent(SecurityDecisionEvent),
    ProfileMutationEvent(ProfileMutationEvent),
}

impl WriteOp {
    /// Ensure a primary emitted event has a stable 12-lower-hex id before it
    /// reaches SQLite. Rule ledger rows already point at a triggering event and
    /// therefore must not mint their own id here.
    pub fn ensure_event_id(&mut self) -> Option<String> {
        match self {
            WriteOp::NetEvent(event) => ensure_option_event_id(&mut event.event_id),
            WriteOp::ModelCall(event) => ensure_option_event_id(&mut event.event_id),
            WriteOp::McpCall(event) => ensure_option_event_id(&mut event.event_id),
            WriteOp::FileEvent(event) => ensure_option_event_id(&mut event.event_id),
            WriteOp::ExecEvent(event) => ensure_option_event_id(&mut event.event_id),
            WriteOp::AuditEvent(event) => ensure_option_event_id(&mut event.event_id),
            WriteOp::DnsEvent(event) => ensure_option_event_id(&mut event.event_id),
            WriteOp::SubstitutionEvent(event) => ensure_option_event_id(&mut event.event_id),
            WriteOp::SecurityRuleEvent(event) => Some(event.event_id.clone()),
            WriteOp::SecurityAskEvent(event) => Some(event.event_id.clone()),
            WriteOp::SecurityDecisionEvent(event) => Some(event.event_id.clone()),
            WriteOp::ProfileMutationEvent(event) => Some(event.mutation_id.clone()),
            WriteOp::ExecEventComplete(_) => None,
        }
    }

    pub fn event_id(&self) -> Option<&str> {
        match self {
            WriteOp::NetEvent(event) => event.event_id.as_deref(),
            WriteOp::ModelCall(event) => event.event_id.as_deref(),
            WriteOp::McpCall(event) => event.event_id.as_deref(),
            WriteOp::FileEvent(event) => event.event_id.as_deref(),
            WriteOp::ExecEvent(event) => event.event_id.as_deref(),
            WriteOp::AuditEvent(event) => event.event_id.as_deref(),
            WriteOp::DnsEvent(event) => event.event_id.as_deref(),
            WriteOp::SubstitutionEvent(event) => event.event_id.as_deref(),
            WriteOp::SecurityRuleEvent(event) => Some(event.event_id.as_str()),
            WriteOp::SecurityAskEvent(event) => Some(event.event_id.as_str()),
            WriteOp::SecurityDecisionEvent(event) => Some(event.event_id.as_str()),
            WriteOp::ProfileMutationEvent(event) => Some(event.mutation_id.as_str()),
            WriteOp::ExecEventComplete(_) => None,
        }
    }
}

fn ensure_option_event_id(event_id: &mut Option<String>) -> Option<String> {
    if event_id.is_none() {
        *event_id = Some(new_event_id());
    }
    event_id.clone()
}

/// A dedicated writer thread that owns the SQLite connection.
///
/// Callers send `WriteOp` values through an mpsc channel. The writer thread
/// blocks until ops arrive, drains the queue, and executes them in a single
/// transaction for efficiency.
///
/// Shutdown is explicit-cleanup safe via `shutdown_blocking(&self)`: callers
/// holding an `Arc<DbWriter>` can deterministically drop the stored sender
/// and join the writer thread without waiting for `Drop` to run when the
/// last Arc clone disappears. This matters under the 1s SIGTERM-to-SIGKILL
/// budget that the service enforces on `capsem-process` teardown -- see
/// /dev-rust-patterns "Signal-driven explicit cleanup".
pub struct DbWriter {
    /// Stored sender. `shutdown_blocking` takes it out; `write` clones it
    /// under the lock and releases the lock before `.await` so hot-path
    /// latency is unaffected. Cloning an mpsc::Sender is cheap (it's an Arc).
    tx: std::sync::Mutex<Option<tokio::sync::mpsc::Sender<WriteOp>>>,
    join_handle: std::sync::Mutex<Option<std::thread::JoinHandle<()>>>,
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
            tx: std::sync::Mutex::new(Some(tx)),
            join_handle: std::sync::Mutex::new(Some(join_handle)),
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
            tx: std::sync::Mutex::new(Some(tx)),
            join_handle: std::sync::Mutex::new(Some(join_handle)),
            db_path: PathBuf::from(":memory:"),
        })
    }

    /// Clone the stored sender so async work can happen outside the lock.
    fn clone_sender(&self) -> Option<tokio::sync::mpsc::Sender<WriteOp>> {
        self.tx.lock().unwrap().clone()
    }

    /// Non-blocking send from async context. Yields if channel full (backpressure).
    pub async fn write(&self, op: WriteOp) {
        let span = tracing::debug_span!(
            target: "capsem.db",
            DB_ENQUEUE_SPAN,
            status = tracing::field::Empty,
            queue_result = tracing::field::Empty,
        );
        let started = Instant::now();
        if let Some(tx) = self.clone_sender() {
            match tx.send(op).instrument(span.clone()).await {
                Ok(()) => {
                    record_enqueue(started, "queued", &span);
                }
                Err(e) => {
                    record_enqueue(started, "closed", &span);
                    warn!(error = %e, "db writer channel closed, dropping write op");
                }
            }
        } else {
            record_enqueue(started, "missing_sender", &span);
        }
    }

    /// Try to send without blocking. Returns false if the channel is full or closed.
    pub fn try_write(&self, op: WriteOp) -> bool {
        let span = tracing::debug_span!(
            target: "capsem.db",
            DB_ENQUEUE_SPAN,
            status = tracing::field::Empty,
            queue_result = tracing::field::Empty,
        );
        let started = Instant::now();
        let accepted = self
            .tx
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|tx| tx.try_send(op).is_ok());
        record_enqueue(
            started,
            if accepted { "queued" } else { "full_or_closed" },
            &span,
        );
        accepted
    }

    /// Blocking send for synchronous producer threads that must not drop
    /// security events. Do not call from Tokio async tasks; async callers
    /// should use `write().await` so the runtime can schedule fairly.
    pub fn write_blocking(&self, op: WriteOp) {
        let span = tracing::debug_span!(
            target: "capsem.db",
            DB_ENQUEUE_SPAN,
            status = tracing::field::Empty,
            queue_result = tracing::field::Empty,
        );
        let started = Instant::now();
        if let Some(tx) = self.clone_sender() {
            if let Err(e) = tx.blocking_send(op) {
                record_enqueue(started, "closed", &span);
                warn!(error = %e, "db writer channel closed, dropping blocking write op");
            } else {
                record_enqueue(started, "queued", &span);
            }
        } else {
            record_enqueue(started, "missing_sender", &span);
        }
    }

    /// Deterministically shut down the writer thread: drop the stored
    /// sender and join. Safe to call through a shared `Arc<DbWriter>` --
    /// other Arc clones stay valid but subsequent `write` calls become
    /// no-ops. Idempotent. Blocks until the writer thread drains its queue
    /// and runs the final `PRAGMA wal_checkpoint(TRUNCATE)`. Call from a
    /// blocking thread (e.g. via `tokio::task::spawn_blocking`).
    ///
    /// Outstanding `write` callers that cloned the sender before this
    /// method ran may still have Sender clones in flight; the join waits
    /// for those clones to drop naturally as their `send().await` returns.
    pub fn shutdown_blocking(&self) {
        let _ = self.tx.lock().unwrap().take();
        let handle = self.join_handle.lock().unwrap().take();
        if let Some(handle) = handle {
            let _ = handle.join();
        }
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
        self.shutdown_blocking();
    }
}

/// The writer thread loop: block-then-drain batching.
fn writer_loop(conn: Connection, mut rx: tokio::sync::mpsc::Receiver<WriteOp>) {
    let mut model_item_dedup = load_model_item_dedup(&conn);

    // 1. Block until at least one op arrives. Returns None when all
    //    Senders are dropped (clean shutdown) and ends the loop.
    while let Some(first_op) = rx.blocking_recv() {
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
        let batch_size = batch.len();
        let batch_bucket = batch_size_bucket(batch_size);
        let span = tracing::debug_span!(
            target: "capsem.db",
            DB_WRITE_BATCH_SPAN,
            batch_size_bucket = batch_bucket,
            status = tracing::field::Empty,
        );
        let started = Instant::now();
        if let Err(e) = span.in_scope(|| execute_batch(&conn, &batch, &mut model_item_dedup)) {
            record_batch(started, batch_size, batch_bucket, "error", &span);
            warn!(error = %e, count = batch.len(), "db write batch failed");
        } else {
            record_batch(started, batch_size, batch_bucket, "ok", &span);
        }
    }

    // Test hook: lets `test_wal_absent_after_clean_shutdown`-style tests
    // simulate a slow checkpoint so the explicit-cleanup path can be
    // distinguished from implicit tokio-runtime-drop ordering. Gated on
    // an env var so it's a no-op in production.
    if let Ok(ms) = std::env::var("CAPSEM_TEST_SLOW_CHECKPOINT_MS") {
        if let Ok(ms) = ms.parse::<u64>() {
            std::thread::sleep(std::time::Duration::from_millis(ms));
        }
    }

    // All senders dropped -- checkpoint WAL before closing connection.
    let span = tracing::debug_span!(
        target: "capsem.db",
        DB_SHUTDOWN_FLUSH_SPAN,
        status = tracing::field::Empty,
    );
    let started = Instant::now();
    let result = span.in_scope(|| conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)"));
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    let status = if result.is_ok() { "ok" } else { "error" };
    ::metrics::histogram!(DB_SHUTDOWN_FLUSH_MS, "status" => status).record(elapsed_ms);
    span.record("status", status);
}

fn load_model_item_dedup(conn: &Connection) -> ModelItemDedup {
    let mut dedup = ModelItemDedup::new();
    let Ok(mut stmt) =
        conn.prepare("SELECT trace_id, kind, content_hash, call_id FROM model_items")
    else {
        return dedup;
    };
    let Ok(rows) = stmt.query_map([], |row| {
        let trace_id: Option<String> = row.get(0)?;
        let kind: String = row.get(1)?;
        let content_hash: String = row.get(2)?;
        let call_id: String = row.get(3)?;
        Ok(model_item_dedup_key(
            trace_id.as_deref(),
            &kind,
            &content_hash,
            &call_id,
        ))
    }) else {
        return dedup;
    };
    for key in rows.flatten() {
        dedup.insert(key);
    }
    dedup
}

fn record_enqueue(started: Instant, queue_result: &'static str, span: &tracing::Span) {
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    ::metrics::histogram!(DB_ENQUEUE_WAIT_MS, "queue_result" => queue_result).record(elapsed_ms);
    span.record(
        "status",
        if queue_result == "queued" {
            "ok"
        } else {
            "error"
        },
    );
    span.record("queue_result", queue_result);
}

fn record_batch(
    started: Instant,
    batch_size: usize,
    batch_size_bucket: &'static str,
    status: &'static str,
    span: &tracing::Span,
) {
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    ::metrics::counter!(DB_WRITE_BATCH_TOTAL,
        "batch_size_bucket" => batch_size_bucket,
        "status" => status)
    .increment(1);
    ::metrics::histogram!(DB_WRITE_BATCH_DURATION_MS,
        "batch_size_bucket" => batch_size_bucket,
        "status" => status)
    .record(elapsed_ms);
    ::metrics::histogram!(DB_WRITE_BATCH_SIZE,
        "batch_size_bucket" => batch_size_bucket)
    .record(batch_size as f64);
    span.record("status", status);
}

fn batch_size_bucket(size: usize) -> &'static str {
    match size {
        0 => "0",
        1 => "1",
        2..=8 => "2_8",
        9..=32 => "9_32",
        33..=128 => "33_128",
        _ => "gt_128",
    }
}

fn execute_batch(
    conn: &Connection,
    batch: &[WriteOp],
    model_item_dedup: &mut ModelItemDedup,
) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    for op in batch {
        match op {
            WriteOp::NetEvent(e) => insert_net_event(&tx, e)?,
            WriteOp::ModelCall(m) => insert_model_call(&tx, m, model_item_dedup)?,
            WriteOp::McpCall(c) => insert_mcp_call(&tx, c)?,
            WriteOp::FileEvent(f) => insert_file_event(&tx, f)?,
            WriteOp::ExecEvent(e) => insert_exec_event(&tx, e)?,
            WriteOp::ExecEventComplete(c) => update_exec_event(&tx, c)?,
            WriteOp::AuditEvent(a) => insert_audit_event(&tx, a)?,
            WriteOp::DnsEvent(d) => insert_dns_event(&tx, d)?,
            WriteOp::SubstitutionEvent(s) => insert_substitution_event(&tx, s)?,
            WriteOp::SecurityRuleEvent(e) => insert_security_rule_event(&tx, e)?,
            WriteOp::SecurityAskEvent(e) => insert_security_ask_event(&tx, e)?,
            WriteOp::SecurityDecisionEvent(e) => insert_security_decision_event(&tx, e)?,
            WriteOp::ProfileMutationEvent(e) => insert_profile_mutation_event(&tx, e)?,
        }
    }
    tx.commit()
}

fn insert_net_event(conn: &Connection, event: &NetEvent) -> rusqlite::Result<()> {
    let timestamp = format_timestamp(event.timestamp);
    let req_body = cap_field(&event.request_body_preview);
    let resp_body = cap_field(&event.response_body_preview);
    let req_headers = cap_field(&event.request_headers);
    let resp_headers = cap_field(&event.response_headers);
    let event_id = event.event_id.clone().unwrap_or_else(new_event_id);
    conn.execute(
        "INSERT INTO net_events (
            event_id, timestamp, domain, port, decision, process_name, pid,
            method, path, query, status_code,
            bytes_sent, bytes_received, duration_ms, matched_rule,
            request_headers, response_headers,
            request_body_preview, response_body_preview, conn_type,
            policy_mode, policy_action, policy_rule, policy_reason,
            trace_id, credential_ref
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26)",
        params![
            event_id,
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
            event.policy_mode,
            event.policy_action,
            event.policy_rule,
            event.policy_reason,
            event.trace_id,
            event.credential_ref,
        ],
    )?;
    insert_event_body_blob(
        conn,
        EventBodyBlob {
            event_id: &event_id,
            event_type: "http.request",
            source_table: "net_events",
            direction: "request",
            content_type: event
                .request_headers
                .as_deref()
                .and_then(content_type_from_headers),
            body: event.request_body_preview.as_deref(),
            trace_id: event.trace_id.as_deref(),
        },
    )?;
    insert_event_body_blob(
        conn,
        EventBodyBlob {
            event_id: &event_id,
            event_type: "http.request",
            source_table: "net_events",
            direction: "response",
            content_type: event
                .response_headers
                .as_deref()
                .and_then(content_type_from_headers),
            body: event.response_body_preview.as_deref(),
            trace_id: event.trace_id.as_deref(),
        },
    )?;
    Ok(())
}

fn insert_model_call(
    conn: &Connection,
    call: &ModelCall,
    model_item_dedup: &mut ModelItemDedup,
) -> rusqlite::Result<()> {
    let timestamp = format_timestamp(call.timestamp);
    let req_body = cap_field(&call.request_body_preview);
    let text_content = cap_field(&call.text_content);
    let thinking_content = cap_field(&call.thinking_content);
    let sys_prompt = cap_field(&call.system_prompt_preview);
    let event_id = call.event_id.clone().unwrap_or_else(new_event_id);
    conn.execute(
        "INSERT INTO model_calls (
            event_id, timestamp, provider, model, process_name, pid,
            method, path, stream,
            system_prompt_preview, messages_count, tools_count,
            request_bytes, request_body_preview,
            message_id, status_code, text_content, thinking_content,
            stop_reason, input_tokens, output_tokens,
            duration_ms, response_bytes, estimated_cost_usd, trace_id,
            usage_details, credential_ref
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27)",
        params![
            event_id,
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
            call.credential_ref,
        ],
    )?;
    let model_call_id = conn.last_insert_rowid();
    insert_event_body_blob(
        conn,
        EventBodyBlob {
            event_id: &event_id,
            event_type: "model.call",
            source_table: "model_calls",
            direction: "request",
            content_type: Some("application/json"),
            body: call.request_body_preview.as_deref(),
            trace_id: call.trace_id.as_deref(),
        },
    )?;
    insert_event_body_blob(
        conn,
        EventBodyBlob {
            event_id: &event_id,
            event_type: "model.call",
            source_table: "model_calls",
            direction: "response",
            content_type: None,
            body: call.text_content.as_deref(),
            trace_id: call.trace_id.as_deref(),
        },
    )?;
    insert_model_items(conn, model_call_id, call, &timestamp, model_item_dedup)?;

    for tc in &call.tool_calls {
        // W6: tool_calls.trace_id falls back to the parent model_call's
        // trace_id (they belong to the same agent turn).
        let tc_trace = tc.trace_id.clone().or_else(|| call.trace_id.clone());
        conn.execute(
            "INSERT INTO tool_calls (
                event_id, model_call_id, provider, status, call_index, call_id,
                tool_name, arguments, origin, trace_id, credential_ref
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                new_event_id(),
                model_call_id,
                call.provider,
                "observed",
                tc.call_index as i64,
                tc.call_id,
                tc.tool_name,
                tc.arguments,
                tc.origin,
                tc_trace,
                call.credential_ref,
            ],
        )?;
    }

    for tr in &call.tool_responses {
        let tr_trace = tr.trace_id.clone().or_else(|| call.trace_id.clone());
        let tr_credential_ref = tr
            .credential_ref
            .clone()
            .or_else(|| call.credential_ref.clone());
        conn.execute(
            "INSERT INTO tool_responses (model_call_id, call_id, content_preview, is_error, trace_id, credential_ref)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                model_call_id,
                tr.call_id,
                tr.content_preview,
                tr.is_error as i64,
                tr_trace,
                tr_credential_ref,
            ],
        )?;
    }

    Ok(())
}

fn insert_model_items(
    conn: &Connection,
    model_call_id: i64,
    call: &ModelCall,
    timestamp: &str,
    model_item_dedup: &mut ModelItemDedup,
) -> rusqlite::Result<()> {
    let mut item_index = 0_i64;
    let mut insert_item = |kind: &str,
                           call_id: Option<&str>,
                           tool_name: Option<&str>,
                           arguments: Option<&str>,
                           content: Option<String>|
     -> rusqlite::Result<()> {
        item_index += 1;
        let call_id = call_id.unwrap_or_default();
        let content = cap_field(&content);
        let hash_material = serde_json::json!({
            "kind": kind,
            "call_id": call_id,
            "tool_name": tool_name,
            "arguments": arguments,
            "content": content,
        })
        .to_string();
        let content_hash = blake3_ref(&hash_material);
        let dedup_key =
            model_item_dedup_key(call.trace_id.as_deref(), kind, &content_hash, call_id);
        if !model_item_dedup.insert(dedup_key) {
            return Ok(());
        }
        conn.execute(
            "INSERT OR IGNORE INTO model_items (
                event_id, model_call_id, timestamp, provider, model, path, trace_id,
                kind, item_index, call_id, tool_name, arguments, content,
                content_hash, credential_ref
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                new_event_id(),
                model_call_id,
                timestamp,
                call.provider,
                call.model,
                call.path,
                call.trace_id,
                kind,
                item_index,
                call_id,
                tool_name,
                arguments,
                content,
                content_hash,
                call.credential_ref,
            ],
        )?;
        Ok(())
    };

    // A tool-result continuation request is represented by tool_response rows;
    // do not also log it as another user request for the same trace.
    if call.tool_responses.is_empty() {
        if let Some(content) = &call.request_body_preview {
            insert_item("request", None, None, None, Some(content.clone()))?;
        }
    }
    if let Some(content) = &call.thinking_content {
        insert_item("reasoning", None, None, None, Some(content.clone()))?;
    }
    if let Some(content) = &call.text_content {
        insert_item("response", None, None, None, Some(content.clone()))?;
    }
    for tool_call in &call.tool_calls {
        insert_item(
            "tool_call",
            Some(&tool_call.call_id),
            Some(&tool_call.tool_name),
            tool_call.arguments.as_deref(),
            tool_call.arguments.clone(),
        )?;
    }
    for tool_response in &call.tool_responses {
        insert_item(
            "tool_response",
            Some(&tool_response.call_id),
            None,
            None,
            tool_response.content_preview.clone(),
        )?;
    }
    Ok(())
}

fn insert_file_event(conn: &Connection, event: &FileEvent) -> rusqlite::Result<()> {
    let timestamp = format_timestamp(event.timestamp);
    let (directory, name) = split_event_path(&event.path);
    conn.execute(
        "INSERT INTO fs_events (event_id, timestamp, action, path, directory, name, size, trace_id, credential_ref)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            event.event_id.clone().unwrap_or_else(new_event_id),
            timestamp,
            event.action.as_str(),
            event.path,
            directory,
            name,
            event.size.map(|s| s as i64),
            event.trace_id,
            event.credential_ref,
        ],
    )?;
    Ok(())
}

fn split_event_path(path: &str) -> (String, String) {
    let normalized = path.trim_end_matches('/');
    if normalized.is_empty() {
        return (".".to_string(), String::new());
    }
    match normalized.rsplit_once('/') {
        Some(("", name)) => ("/".to_string(), name.to_string()),
        Some((dir, name)) if !name.is_empty() => (dir.to_string(), name.to_string()),
        _ => (".".to_string(), normalized.to_string()),
    }
}

fn insert_mcp_call(conn: &Connection, call: &McpCall) -> rusqlite::Result<()> {
    let timestamp = format_timestamp(call.timestamp);
    let req_preview = cap_field(&call.request_preview);
    let resp_preview = cap_field(&call.response_preview);
    let event_id = call.event_id.clone().unwrap_or_else(new_event_id);
    conn.execute(
        "INSERT INTO mcp_calls (
            event_id, timestamp, server_name, method, tool_name, request_id,
            request_preview, response_preview, decision,
            duration_ms, error_message, process_name,
            bytes_sent, bytes_received,
            policy_mode, policy_action, policy_rule, policy_reason,
            trace_id, credential_ref
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
        params![
            event_id,
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
            call.policy_mode,
            call.policy_action,
            call.policy_rule,
            call.policy_reason,
            call.trace_id,
            call.credential_ref,
        ],
    )?;
    let event_type = if call.method == "tools/list" {
        "mcp.tool_list"
    } else if call.method == "tools/call" {
        "mcp.tool_call"
    } else {
        "mcp.event"
    };
    insert_event_body_blob(
        conn,
        EventBodyBlob {
            event_id: &event_id,
            event_type,
            source_table: "mcp_calls",
            direction: "request",
            content_type: Some("application/json"),
            body: call.request_preview.as_deref(),
            trace_id: call.trace_id.as_deref(),
        },
    )?;
    insert_event_body_blob(
        conn,
        EventBodyBlob {
            event_id: &event_id,
            event_type,
            source_table: "mcp_calls",
            direction: "response",
            content_type: Some("application/json"),
            body: call.response_preview.as_deref(),
            trace_id: call.trace_id.as_deref(),
        },
    )?;
    Ok(())
}

fn content_type_from_headers(headers: &str) -> Option<&str> {
    headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.trim().eq_ignore_ascii_case("content-type") {
            Some(value.trim())
        } else {
            None
        }
    })
}

struct EventBodyBlob<'a> {
    event_id: &'a str,
    event_type: &'a str,
    source_table: &'a str,
    direction: &'a str,
    content_type: Option<&'a str>,
    body: Option<&'a str>,
    trace_id: Option<&'a str>,
}

fn insert_event_body_blob(conn: &Connection, blob: EventBodyBlob<'_>) -> rusqlite::Result<()> {
    let Some(body) = blob.body else {
        return Ok(());
    };
    if body.is_empty() {
        return Ok(());
    }
    let bytes = body.as_bytes();
    let stored_len = bytes.len().min(MAX_BODY_BLOB_BYTES);
    let stored = &bytes[..stored_len];
    let created_at = format_timestamp(SystemTime::now());
    conn.execute(
        "INSERT OR REPLACE INTO event_body_blobs (
            event_id, event_type, source_table, direction, content_type,
            original_bytes, stored_bytes, truncated, body_hash, body,
            trace_id, created_at
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            blob.event_id,
            blob.event_type,
            blob.source_table,
            blob.direction,
            blob.content_type,
            bytes.len() as i64,
            stored_len as i64,
            (bytes.len() > stored_len) as i64,
            blake3_bytes_ref(bytes),
            stored,
            blob.trace_id,
            created_at,
        ],
    )?;
    Ok(())
}

fn insert_exec_event(conn: &Connection, event: &ExecEvent) -> rusqlite::Result<()> {
    let timestamp = format_timestamp(event.timestamp);
    conn.execute(
        "INSERT INTO exec_events (
            event_id, timestamp, exec_id, command, source, mcp_call_id, trace_id, process_name, credential_ref
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            event.event_id.clone().unwrap_or_else(new_event_id),
            timestamp,
            event.exec_id as i64,
            event.command,
            event.source,
            event.mcp_call_id.map(|id| id as i64),
            event.trace_id,
            event.process_name,
            event.credential_ref,
        ],
    )?;
    Ok(())
}

fn update_exec_event(conn: &Connection, complete: &ExecEventComplete) -> rusqlite::Result<()> {
    let stdout_preview = cap_field(&complete.stdout_preview);
    let stderr_preview = cap_field(&complete.stderr_preview);
    conn.execute(
        "UPDATE exec_events SET
            exit_code = ?1,
            duration_ms = ?2,
            stdout_preview = ?3,
            stderr_preview = ?4,
            stdout_bytes = ?5,
            stderr_bytes = ?6,
            pid = ?7
         WHERE exec_id = ?8",
        params![
            complete.exit_code as i64,
            complete.duration_ms as i64,
            stdout_preview,
            stderr_preview,
            complete.stdout_bytes as i64,
            complete.stderr_bytes as i64,
            complete.pid.map(|p| p as i64),
            complete.exec_id as i64,
        ],
    )?;
    Ok(())
}

fn insert_dns_event(conn: &Connection, event: &DnsEvent) -> rusqlite::Result<()> {
    let timestamp = format_timestamp(event.timestamp);
    conn.execute(
        "INSERT INTO dns_events (
            event_id, timestamp, qname, qtype, qclass, rcode, decision, matched_rule,
            answer_ip, source_proto, process_name, upstream_resolver_ms, trace_id,
            policy_mode, policy_action, policy_rule, policy_reason, credential_ref
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
        params![
            event.event_id.clone().unwrap_or_else(new_event_id),
            timestamp,
            event.qname,
            event.qtype as i64,
            event.qclass as i64,
            event.rcode as i64,
            event.decision,
            event.matched_rule,
            event.answer_ip,
            event.source_proto,
            event.process_name,
            event.upstream_resolver_ms as i64,
            event.trace_id,
            event.policy_mode,
            event.policy_action,
            event.policy_rule,
            event.policy_reason,
            event.credential_ref,
        ],
    )?;
    Ok(())
}

fn insert_audit_event(conn: &Connection, event: &AuditEvent) -> rusqlite::Result<()> {
    let timestamp = format_timestamp(event.timestamp);
    conn.execute(
        "INSERT INTO audit_events (
            event_id, timestamp, pid, ppid, uid, exe, comm, argv, cwd,
            session_id, tty, audit_id, exec_event_id, parent_exe, trace_id, credential_ref
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        params![
            event.event_id.clone().unwrap_or_else(new_event_id),
            timestamp,
            event.pid as i64,
            event.ppid as i64,
            event.uid as i64,
            event.exe,
            event.comm,
            event.argv,
            event.cwd,
            event.session_id.map(|s| s as i64),
            event.tty,
            event.audit_id,
            event.exec_event_id,
            event.parent_exe,
            event.trace_id,
            event.credential_ref,
        ],
    )?;
    Ok(())
}

fn insert_substitution_event(conn: &Connection, event: &SubstitutionEvent) -> rusqlite::Result<()> {
    let timestamp = format_timestamp(event.timestamp);
    conn.execute(
        "INSERT INTO substitution_events (
            event_id, timestamp, material_class, source, event_type, algorithm,
            substitution_ref, outcome, provider, confidence, trace_id, context_json
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            event.event_id.clone().unwrap_or_else(new_event_id),
            timestamp,
            event.material_class,
            event.source,
            event.event_type,
            event.algorithm,
            event.substitution_ref,
            event.outcome,
            event.provider,
            event.confidence,
            event.trace_id,
            event.context_json,
        ],
    )?;
    Ok(())
}

fn insert_security_rule_event(
    conn: &Connection,
    event: &SecurityRuleEvent,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO security_rule_events (
            timestamp_unix_ms, event_id, event_type, rule_id,
            rule_action, detection_level, rule_json, event_json, trace_id
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            event.timestamp_unix_ms,
            event.event_id,
            event.event_type,
            event.rule_id,
            event.rule_action.as_str(),
            event.detection_level.as_str(),
            event.rule_json,
            event.event_json,
            event.trace_id,
        ],
    )?;
    Ok(())
}

fn insert_security_ask_event(conn: &Connection, event: &SecurityAskEvent) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO security_ask_events (
            timestamp_unix_ms, ask_id, event_id, event_type, rule_id, rule_name,
            status, rule_json, event_json, resolver, reason, trace_id
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            event.timestamp_unix_ms,
            event.ask_id,
            event.event_id,
            event.event_type,
            event.rule_id,
            event.rule_name,
            event.status.as_str(),
            event.rule_json,
            event.event_json,
            event.resolver,
            event.reason,
            event.trace_id,
        ],
    )?;
    Ok(())
}

fn insert_security_decision_event(
    conn: &Connection,
    event: &SecurityDecisionEvent,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO security_decision_events (
            timestamp_unix_ms, event_id, event_type, stage, actor,
            rule_id, plugin_id, previous_decision, requested_decision,
            effective_decision, reason, event_json, trace_id
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            event.timestamp_unix_ms,
            event.event_id,
            event.event_type,
            event.stage.as_str(),
            event.actor,
            event.rule_id,
            event.plugin_id,
            event.previous_decision.as_str(),
            event.requested_decision.as_str(),
            event.effective_decision.as_str(),
            event.reason,
            event.event_json,
            event.trace_id,
        ],
    )?;
    Ok(())
}

fn insert_profile_mutation_event(
    conn: &Connection,
    event: &ProfileMutationEvent,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO profile_mutation_events (
            timestamp_unix_ms, mutation_id, profile_id, actor, category, filename,
            affected_path, target_kind, target_key, operation, rule_id,
            old_hash, old_size, new_hash, new_size, status, error, trace_id
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
        params![
            event.timestamp_unix_ms,
            event.mutation_id,
            event.profile_id,
            event.actor,
            event.category,
            event.filename,
            event.affected_path,
            event.target_kind,
            event.target_key,
            event.operation,
            event.rule_id,
            event.old_hash,
            event.old_size as i64,
            event.new_hash,
            event.new_size as i64,
            event.status.as_str(),
            event.error,
            event.trace_id,
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;
