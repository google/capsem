use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};
use tracing::warn;

use crate::events::{
    AuditEvent, DnsEvent, ExecEvent, ExecEventComplete, FileEvent, McpCall, ModelCall, NetEvent,
    PolicyHookEvent, SnapshotEvent, TelemetryIdentity,
};
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
    ExecEvent(ExecEvent),
    ExecEventComplete(ExecEventComplete),
    AuditEvent(AuditEvent),
    DnsEvent(DnsEvent),
    PolicyHookEvent(PolicyHookEvent),
    TelemetryIdentity(TelemetryIdentity),
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
        if let Some(tx) = self.clone_sender() {
            if let Err(e) = tx.send(op).await {
                warn!(error = %e, "db writer channel closed, dropping write op");
            }
        }
    }

    /// Try to send without blocking. Returns false if the channel is full or closed.
    pub fn try_write(&self, op: WriteOp) -> bool {
        self.tx
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|tx| tx.try_send(op).is_ok())
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
        if let Err(e) = execute_batch(&conn, &batch) {
            warn!(error = %e, count = batch.len(), "db write batch failed");
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
            WriteOp::ExecEvent(e) => insert_exec_event(&tx, e)?,
            WriteOp::ExecEventComplete(c) => update_exec_event(&tx, c)?,
            WriteOp::AuditEvent(a) => insert_audit_event(&tx, a)?,
            WriteOp::DnsEvent(d) => insert_dns_event(&tx, d)?,
            WriteOp::PolicyHookEvent(h) => insert_policy_hook_event(&tx, h)?,
            WriteOp::TelemetryIdentity(i) => insert_telemetry_identity(&tx, i)?,
        }
    }
    tx.commit()
}

fn insert_telemetry_identity(
    conn: &Connection,
    identity: &TelemetryIdentity,
) -> rusqlite::Result<()> {
    let timestamp = humantime::format_rfc3339(identity.timestamp).to_string();
    conn.execute(
        "INSERT INTO session_identity (id, updated_at, vm_id, profile_id, user_id)
         VALUES (1, ?1, ?2, ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET
            updated_at = excluded.updated_at,
            vm_id = excluded.vm_id,
            profile_id = excluded.profile_id,
            user_id = excluded.user_id",
        params![
            timestamp,
            identity.vm_id,
            identity.profile_id,
            identity.user_id,
        ],
    )?;
    Ok(())
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
            request_body_preview, response_body_preview, conn_type,
            policy_mode, policy_action, policy_rule, policy_reason,
            trace_id
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)",
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
            event.policy_mode,
            event.policy_action,
            event.policy_rule,
            event.policy_reason,
            event.trace_id,
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
        // W6: tool_calls.trace_id falls back to the parent model_call's
        // trace_id (they belong to the same agent turn).
        let tc_trace = tc.trace_id.clone().or_else(|| call.trace_id.clone());
        conn.execute(
            "INSERT INTO tool_calls (model_call_id, call_index, call_id, tool_name, arguments, origin, trace_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                model_call_id,
                tc.call_index as i64,
                tc.call_id,
                tc.tool_name,
                tc.arguments,
                tc.origin,
                tc_trace,
            ],
        )?;
    }

    for tr in &call.tool_responses {
        let tr_trace = tr.trace_id.clone().or_else(|| call.trace_id.clone());
        conn.execute(
            "INSERT INTO tool_responses (model_call_id, call_id, content_preview, is_error, trace_id)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                model_call_id,
                tr.call_id,
                tr.content_preview,
                tr.is_error as i64,
                tr_trace,
            ],
        )?;
    }

    Ok(())
}

fn insert_file_event(conn: &Connection, event: &FileEvent) -> rusqlite::Result<()> {
    let timestamp = humantime::format_rfc3339(event.timestamp).to_string();
    conn.execute(
        "INSERT INTO fs_events (timestamp, action, path, size, trace_id)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            timestamp,
            event.action.as_str(),
            event.path,
            event.size.map(|s| s as i64),
            event.trace_id,
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
            bytes_sent, bytes_received,
            policy_mode, policy_action, policy_rule, policy_reason,
            trace_id
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
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
            call.policy_mode,
            call.policy_action,
            call.policy_rule,
            call.policy_reason,
            call.trace_id,
        ],
    )?;
    Ok(())
}

fn insert_snapshot_event(conn: &Connection, event: &SnapshotEvent) -> rusqlite::Result<()> {
    let timestamp = humantime::format_rfc3339(event.timestamp).to_string();
    conn.execute(
        "INSERT INTO snapshot_events (
            timestamp, slot, origin, name, files_count,
            start_fs_event_id, stop_fs_event_id, trace_id
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            timestamp,
            event.slot as i64,
            event.origin,
            event.name,
            event.files_count as i64,
            event.start_fs_event_id,
            event.stop_fs_event_id,
            event.trace_id,
        ],
    )?;
    Ok(())
}

fn insert_exec_event(conn: &Connection, event: &ExecEvent) -> rusqlite::Result<()> {
    let timestamp = humantime::format_rfc3339(event.timestamp).to_string();
    conn.execute(
        "INSERT INTO exec_events (
            timestamp, exec_id, command, source, mcp_call_id, trace_id, process_name
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            timestamp,
            event.exec_id as i64,
            event.command,
            event.source,
            event.mcp_call_id.map(|id| id as i64),
            event.trace_id,
            event.process_name,
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
    let timestamp = humantime::format_rfc3339(event.timestamp).to_string();
    conn.execute(
        "INSERT INTO dns_events (
            timestamp, qname, qtype, qclass, rcode, decision, matched_rule,
            source_proto, process_name, upstream_resolver_ms, trace_id,
            policy_mode, policy_action, policy_rule, policy_reason
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        params![
            timestamp,
            event.qname,
            event.qtype as i64,
            event.qclass as i64,
            event.rcode as i64,
            event.decision,
            event.matched_rule,
            event.source_proto,
            event.process_name,
            event.upstream_resolver_ms as i64,
            event.trace_id,
            event.policy_mode,
            event.policy_action,
            event.policy_rule,
            event.policy_reason,
        ],
    )?;
    Ok(())
}

fn insert_policy_hook_event(conn: &Connection, event: &PolicyHookEvent) -> rusqlite::Result<()> {
    let timestamp = humantime::format_rfc3339(event.timestamp).to_string();
    let reason = cap_field(&event.reason);
    let error = cap_field(&event.error);
    let audit_tags = if event.audit_tags.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&event.audit_tags).unwrap_or_default())
    };
    conn.execute(
        "INSERT INTO policy_hook_events (
            timestamp, endpoint_id, spec_version, spec_hash, decision_id,
            callback, decision, rule_id, reason, latency_ms, status, error,
            fallback, audit_tags, trace_id, session_id
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        params![
            timestamp,
            event.endpoint_id,
            event.spec_version,
            event.spec_hash,
            event.decision_id,
            event.callback,
            event.decision,
            event.rule_id,
            reason,
            event.latency_ms as i64,
            event.status,
            error,
            event.fallback,
            audit_tags,
            event.trace_id,
            event.session_id,
        ],
    )?;
    Ok(())
}

fn insert_audit_event(conn: &Connection, event: &AuditEvent) -> rusqlite::Result<()> {
    let timestamp = humantime::format_rfc3339(event.timestamp).to_string();
    conn.execute(
        "INSERT INTO audit_events (
            timestamp, pid, ppid, uid, exe, comm, argv, cwd,
            session_id, tty, audit_id, exec_event_id, parent_exe, trace_id
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
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
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;
