//! Rebuild the checked-in session fixture in the current ledger shape.
//!
//! The fixture is a real recorded session, and it is shared by the frontend's
//! mock mode and by these tests, so it must stay real *and* current. Editing
//! the binary in place with an `ALTER TABLE` was how it stayed current once;
//! that hides a schema change inside an opaque blob nobody reviews.
//!
//! This replays every row through the same `WriteOp` path production uses, so
//! the regenerated file has whatever shape the writer publishes today and the
//! content is the content that was recorded. Run it deliberately:
//!
//! ```text
//! cargo test -p capsem-logger --test roundtrip -- --ignored regenerate_session_fixture
//! ```
//!
//! Then commit the new binary together with its `sha256` in
//! `tests/citadel/fixture_ownership.toml`, which is what makes the edit a
//! reviewed act rather than a silent one.
//!
//! Rows the current ledger has no home for are dropped, and the run says so:
//! the old fixture carries a `snapshot_events` table that no longer exists in
//! the schema. The recorded request and response text does survive, but not
//! where it used to live: the current writer puts bodies in the block archive,
//! so the replay produces a `test.bodies` beside the ledger and both files are
//! committed together.

use super::*;

use rusqlite::Connection;

fn fixture_path() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop();
    path.pop();
    path.push("tests/fixtures/session/test.db");
    path
}

/// Read a column that may not exist in the shape being read.
///
/// The replay has to work on the shape it is upgrading *from* and on the shape
/// it produces, or it is a one-shot script that rots the day after it runs.
fn opt<T: rusqlite::types::FromSql>(row: &rusqlite::Row<'_>, column: &str) -> Option<T> {
    row.get::<_, Option<T>>(column).ok().flatten()
}

fn at(row: &rusqlite::Row<'_>, column: &str) -> SystemTime {
    opt::<String>(row, column)
        .and_then(|value| humantime::parse_rfc3339(&value).ok())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

fn table_exists(conn: &Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [table],
        |_| Ok(()),
    )
    .is_ok()
}

fn decision_of(value: &str) -> Decision {
    match value {
        "denied" => Decision::Denied,
        "error" => Decision::Error,
        "redirected" => Decision::Redirected,
        _ => Decision::Allowed,
    }
}

fn replay_net_events(conn: &Connection, writer: &DbWriter) -> usize {
    let mut stmt = conn.prepare("SELECT * FROM net_events ORDER BY id").unwrap();
    let events = stmt
        .query_map([], |row| {
            Ok(NetEvent {
                event_id: opt(row, "event_id"),
                timestamp: at(row, "timestamp"),
                domain: opt(row, "domain").unwrap_or_default(),
                port: opt::<i64>(row, "port").unwrap_or(443) as u16,
                decision: decision_of(&opt::<String>(row, "decision").unwrap_or_default()),
                process_name: opt(row, "process_name"),
                pid: opt::<i64>(row, "pid").map(|v| v as u32),
                method: opt(row, "method"),
                path: opt(row, "path"),
                query: opt(row, "query"),
                status_code: opt::<i64>(row, "status_code").map(|v| v as u16),
                bytes_sent: opt::<i64>(row, "bytes_sent").unwrap_or_default() as u64,
                bytes_received: opt::<i64>(row, "bytes_received").unwrap_or_default() as u64,
                duration_ms: opt::<i64>(row, "duration_ms").unwrap_or_default() as u64,
                matched_rule: opt(row, "matched_rule"),
                request_headers: opt(row, "request_headers"),
                response_headers: opt(row, "response_headers"),
                request_body_preview: opt(row, "request_body_preview"),
                response_body_preview: opt(row, "response_body_preview"),
                request_body_full: opt(row, "request_body_full"),
                response_body_full: opt(row, "response_body_full"),
                conn_type: opt(row, "conn_type"),
                policy_mode: opt(row, "policy_mode"),
                policy_action: opt(row, "policy_action"),
                policy_rule: opt(row, "policy_rule"),
                policy_reason: opt(row, "policy_reason"),
                trace_id: opt(row, "trace_id"),
                credential_ref: opt(row, "credential_ref"),
            })
        })
        .unwrap()
        .map(Result::unwrap)
        .collect::<Vec<_>>();
    let count = events.len();
    for event in events {
        writer.write_blocking(WriteOp::NetEvent(event));
    }
    count
}

fn tool_calls_for(conn: &Connection, model_call_id: i64) -> Vec<ToolCallEntry> {
    let mut stmt = conn
        .prepare("SELECT * FROM tool_calls WHERE model_call_id = ?1 ORDER BY call_index")
        .unwrap();
    stmt.query_map([model_call_id], |row| {
        Ok(ToolCallEntry {
            call_index: opt::<i64>(row, "call_index").unwrap_or_default() as u32,
            call_id: opt(row, "call_id").unwrap_or_default(),
            tool_name: opt(row, "tool_name").unwrap_or_default(),
            arguments: opt(row, "arguments"),
            origin: opt(row, "origin").unwrap_or_else(|| "native".to_string()),
            trace_id: opt(row, "trace_id"),
        })
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

fn tool_responses_for(conn: &Connection, model_call_id: i64) -> Vec<ToolResponseEntry> {
    let mut stmt = conn
        .prepare("SELECT * FROM tool_responses WHERE model_call_id = ?1 ORDER BY id")
        .unwrap();
    stmt.query_map([model_call_id], |row| {
        Ok(ToolResponseEntry {
            call_id: opt(row, "call_id").unwrap_or_default(),
            content_preview: opt(row, "content_preview"),
            is_error: opt::<i64>(row, "is_error").unwrap_or_default() != 0,
            trace_id: opt(row, "trace_id"),
            credential_ref: opt(row, "credential_ref"),
        })
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

fn replay_model_calls(conn: &Connection, writer: &DbWriter) -> usize {
    let mut stmt = conn.prepare("SELECT * FROM model_calls ORDER BY id").unwrap();
    let calls = stmt
        .query_map([], |row| {
            let id: i64 = row.get("id")?;
            Ok((
                id,
                ModelCall {
                    event_id: opt(row, "event_id"),
                    timestamp: at(row, "timestamp"),
                    provider: opt(row, "provider").unwrap_or_default(),
                    protocol: opt(row, "protocol"),
                    model: opt(row, "model"),
                    process_name: opt(row, "process_name"),
                    pid: opt::<i64>(row, "pid").map(|v| v as u32),
                    method: opt(row, "method").unwrap_or_default(),
                    path: opt(row, "path").unwrap_or_default(),
                    stream: opt::<i64>(row, "stream").unwrap_or_default() != 0,
                    system_prompt_preview: opt(row, "system_prompt_preview"),
                    messages_count: opt::<i64>(row, "messages_count").unwrap_or_default() as usize,
                    tools_count: opt::<i64>(row, "tools_count").unwrap_or_default() as usize,
                    request_bytes: opt::<i64>(row, "request_bytes").unwrap_or_default() as u64,
                    request_body_preview: opt(row, "request_body_preview"),
                    request_body_full: opt(row, "request_body_full"),
                    message_id: opt(row, "message_id"),
                    status_code: opt::<i64>(row, "status_code").map(|v| v as u16),
                    text_content: opt(row, "text_content"),
                    thinking_content: opt(row, "thinking_content"),
                    response_body_full: opt(row, "response_body_full"),
                    stop_reason: opt(row, "stop_reason"),
                    input_tokens: opt::<i64>(row, "input_tokens").map(|v| v as u64),
                    output_tokens: opt::<i64>(row, "output_tokens").map(|v| v as u64),
                    usage_details: opt::<String>(row, "usage_details")
                        .and_then(|value| serde_json::from_str(&value).ok())
                        .unwrap_or_default(),
                    duration_ms: opt::<i64>(row, "duration_ms").unwrap_or_default() as u64,
                    response_bytes: opt::<i64>(row, "response_bytes").unwrap_or_default() as u64,
                    estimated_cost_usd: opt::<f64>(row, "estimated_cost_usd").unwrap_or_default(),
                    trace_id: opt(row, "trace_id"),
                    credential_ref: opt(row, "credential_ref"),
                    tool_calls: Vec::new(),
                    tool_responses: Vec::new(),
                },
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect::<Vec<_>>();
    let count = calls.len();
    for (id, mut call) in calls {
        call.tool_calls = tool_calls_for(conn, id);
        call.tool_responses = tool_responses_for(conn, id);
        writer.write_blocking(WriteOp::ModelCall(call));
    }
    count
}

fn replay_mcp_calls(conn: &Connection, writer: &DbWriter) -> usize {
    // The old fixture keeps MCP evidence in its own table; the current ledger
    // keeps it in `tool_calls` with `origin = 'mcp'`.
    let sql = if table_exists(conn, "mcp_calls") {
        "SELECT * FROM mcp_calls ORDER BY id"
    } else {
        "SELECT * FROM tool_calls WHERE origin = 'mcp' ORDER BY id"
    };
    let mut stmt = conn.prepare(sql).unwrap();
    let calls = stmt
        .query_map([], |row| {
            Ok(McpCall {
                event_id: opt(row, "event_id"),
                timestamp: at(row, "timestamp"),
                server_name: opt(row, "server_name").unwrap_or_default(),
                method: opt(row, "method").unwrap_or_default(),
                tool_name: opt(row, "tool_name"),
                request_id: opt(row, "request_id"),
                request_preview: opt(row, "request_preview").or_else(|| opt(row, "arguments")),
                response_preview: opt(row, "response_preview"),
                decision: opt(row, "decision").unwrap_or_else(|| "allowed".to_string()),
                duration_ms: opt::<i64>(row, "duration_ms").unwrap_or_default() as u64,
                error_message: opt(row, "error_message"),
                process_name: opt(row, "process_name"),
                bytes_sent: opt::<i64>(row, "bytes_sent").unwrap_or_default() as u64,
                bytes_received: opt::<i64>(row, "bytes_received").unwrap_or_default() as u64,
                transport: opt(row, "transport").unwrap_or_else(|| "unknown".to_string()),
                policy_mode: opt(row, "policy_mode"),
                policy_action: opt(row, "policy_action"),
                policy_rule: opt(row, "policy_rule"),
                policy_reason: opt(row, "policy_reason"),
                trace_id: opt(row, "trace_id"),
                credential_ref: opt(row, "credential_ref"),
            })
        })
        .unwrap()
        .map(Result::unwrap)
        .collect::<Vec<_>>();
    let count = calls.len();
    for call in calls {
        writer.write_blocking(WriteOp::McpCall(call));
    }
    count
}

fn replay_file_events(conn: &Connection, writer: &DbWriter) -> usize {
    let mut stmt = conn.prepare("SELECT * FROM fs_events ORDER BY id").unwrap();
    let events = stmt
        .query_map([], |row| {
            Ok(FileEvent {
                event_id: opt(row, "event_id"),
                timestamp: at(row, "timestamp"),
                action: FileAction::parse_str(&opt::<String>(row, "action").unwrap_or_default()),
                path: opt(row, "path").unwrap_or_default(),
                size: opt::<i64>(row, "size").map(|v| v as u64),
                // The session predates `kind`. Every path it recorded was a
                // file: the monitor of the day did not report directories at
                // all, so `file` is what was observed, not a guess.
                kind: opt::<String>(row, "kind")
                    .map(|value| match value.as_str() {
                        "dir" => FileKind::Dir,
                        "symlink" => FileKind::Symlink,
                        "other" => FileKind::Other,
                        _ => FileKind::File,
                    })
                    .unwrap_or(FileKind::File),
                trace_id: opt(row, "trace_id"),
                credential_ref: opt(row, "credential_ref"),
            })
        })
        .unwrap()
        .map(Result::unwrap)
        .collect::<Vec<_>>();
    let count = events.len();
    for event in events {
        writer.write_blocking(WriteOp::FileEvent(event));
    }
    count
}

#[test]
#[ignore = "rewrites the checked-in fixture; run deliberately, then commit the binary and its sha256"]
fn regenerate_session_fixture() {
    let fixture = fixture_path();
    let source = Connection::open_with_flags(&fixture, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .expect("the existing fixture must be readable");

    let staging = tempfile::tempdir().unwrap();
    let rebuilt = staging.path().join("session.db");
    let writer = DbWriter::open(&rebuilt, 256).unwrap();

    let net = replay_net_events(&source, &writer);
    let model = replay_model_calls(&source, &writer);
    let mcp = replay_mcp_calls(&source, &writer);
    let files = replay_file_events(&source, &writer);
    writer.shutdown_blocking();

    if table_exists(&source, "snapshot_events") {
        let dropped: i64 = source
            .query_row("SELECT COUNT(*) FROM snapshot_events", [], |row| row.get(0))
            .unwrap();
        println!("dropped {dropped} snapshot_events rows: the current ledger has no such table");
    }
    println!("replayed net={net} model={model} mcp={mcp} files={files}");
    assert!(
        net > 0 && model > 0 && files > 0,
        "the fixture must stay a real session"
    );

    // Only the ledger file is the fixture; a WAL left beside it would make the
    // committed blob depend on whether a checkpoint had run.
    assert!(
        !rebuilt.with_extension("db-wal").exists(),
        "shutdown must checkpoint before the file is copied"
    );
    // The old fixture kept request and response text in the ledger's own
    // preview columns. The current writer puts bodies in the block archive and
    // keeps only the index, so replaying that same content produces a
    // `session.bodies` beside the ledger -- and an index pointing into an
    // archive nobody committed would be worse than no fixture at all.
    let indexed_bodies: i64 = Connection::open_with_flags(&rebuilt, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM event_body_blobs", [], |row| row.get(0))
        .unwrap();
    let rebuilt_bodies = rebuilt.with_extension("bodies");
    let fixture_bodies = fixture.with_extension("bodies");
    println!("indexed bodies: {indexed_bodies}");
    std::fs::copy(&rebuilt, &fixture).expect("write the regenerated fixture");
    if indexed_bodies > 0 {
        std::fs::copy(&rebuilt_bodies, &fixture_bodies).expect("write the regenerated body archive");
    } else if fixture_bodies.exists() {
        std::fs::remove_file(&fixture_bodies).unwrap();
    }

    let reader = DbReader::open_disk_only(&fixture).unwrap();
    reader
        .ready()
        .expect("the regenerated fixture must be a current-shape ledger");
}
