//! Rebuild the checked-in session fixture in the current ledger shape.
//!
//! The fixture is a real recorded session, read by the roundtrip tests named
//! in `tests/citadel/fixture_ownership.toml`, so it must stay real *and*
//! current. Editing the binary in place with an `ALTER TABLE` was how it
//! stayed current once; that hides a schema change inside an opaque blob
//! nobody reviews.
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
//! Two rails lose rows on the way, both by design, and the run counts each one
//! read against each one persisted so that a third kind of loss cannot pass
//! unnoticed:
//!
//! - **`snapshot_events`**: the table is gone from the schema. Its 5 rows have
//!   nowhere to land and are dropped whole.
//! - **MCP**: the old fixture logged every protocol frame; the current ledger
//!   records tool invocations, so only `tools/call` frames persist. 21 rows in,
//!   7 out. The replay asserts the survivors are exactly the `tools/call` rows
//!   rather than waiving the difference.
//!
//! The recorded request and response text does survive, but not where it used
//! to live: the current writer puts bodies in the block archive, so the replay
//! produces a `test.bodies` beside the ledger and both files are committed
//! together.
//!
//! One limitation, checked rather than assumed: the replay reads the ledger's
//! own columns, and the current ledger keeps only display excerpts there. That
//! is lossless for the legacy fixture, whose excerpts were the whole of its
//! content, but it means this replay cannot regenerate a fixture it has
//! already produced -- it would write the excerpts back as the bodies and
//! shorten them. The run compares body bytes in against body bytes out and
//! refuses instead of degrading, and the message says the fix: read the
//! archive through `DbHandle::read_body` and restore the full bodies. Do that
//! before the next schema change needs this fixture rebuilt.

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

/// One rail's account of the replay.
///
/// `read` and `persisted` are counted on opposite sides, because the number of
/// rows handed to the writer is not the number the writer keeps: the log used
/// to print what was read and call it what was written, which made a 14-row
/// drop look like a clean run.
struct Rail {
    name: &'static str,
    read: usize,
    /// What the current ledger should hold, derived from the source rather
    /// than asserted as a constant.
    expected: i64,
    persisted: i64,
    /// Why this rail may keep fewer rows than it read. A rail that loses rows
    /// without one is a bug, not a policy.
    declared_drop: Option<&'static str>,
}

impl Rail {
    fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if self.persisted != self.expected {
            problems.push(format!(
                "{}: expected {} rows in the rebuilt ledger, found {}",
                self.name, self.expected, self.persisted
            ));
        }
        if self.expected < self.read as i64 && self.declared_drop.is_none() {
            problems.push(format!(
                "{}: {} of {} rows dropped, and no declared reason says why",
                self.name,
                self.read as i64 - self.expected,
                self.read
            ));
        }
        problems
    }

    fn report(&self) -> String {
        let mut line = format!("{}: {} read -> {} persisted", self.name, self.read, self.persisted);
        if let (true, Some(reason)) = (self.expected < self.read as i64, self.declared_drop) {
            line.push_str(&format!(" ({reason})"));
        }
        line
    }
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get(0)).unwrap_or(0)
}

/// The accounting is the part of the regeneration that can rot silently: it
/// only runs when someone rebuilds the fixture, so it gets its adversarial
/// case here, in the suite that runs every time.
#[test]
fn rail_accounting_reports_undeclared_drops_only() {
    let clean = Rail {
        name: "net_events",
        read: 7,
        expected: 7,
        persisted: 7,
        declared_drop: None,
    };
    assert!(clean.problems().is_empty());
    assert_eq!(clean.report(), "net_events: 7 read -> 7 persisted");

    let declared = Rail {
        name: "mcp",
        read: 21,
        expected: 7,
        persisted: 7,
        declared_drop: Some("only tool invocations"),
    };
    assert!(declared.problems().is_empty());
    assert!(declared.report().ends_with("(only tool invocations)"));

    // The shape the old log hid: rows read, fewer kept, nobody said why.
    let silent = Rail {
        read: 21,
        expected: 21,
        persisted: 7,
        declared_drop: None,
        ..clean
    };
    assert_eq!(
        silent.problems(),
        vec!["net_events: expected 21 rows in the rebuilt ledger, found 7"]
    );

    // A drop the replay expects but nobody declared is equally a bug.
    let undeclared = Rail {
        read: 21,
        expected: 7,
        persisted: 7,
        declared_drop: None,
        ..clean
    };
    assert_eq!(
        undeclared.problems(),
        vec!["net_events: 14 of 21 rows dropped, and no declared reason says why"]
    );
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

    let native_tool_calls = count(&source, "SELECT COUNT(*) FROM tool_calls WHERE origin != 'mcp'");
    let tool_responses = count(&source, "SELECT COUNT(*) FROM tool_responses");
    // Only tool invocations survive the MCP rail, so the expectation is
    // computed from the source rather than waived.
    let mcp_tool_calls = if table_exists(&source, "mcp_calls") {
        count(&source, "SELECT COUNT(*) FROM mcp_calls WHERE method = 'tools/call'")
    } else {
        count(
            &source,
            "SELECT COUNT(*) FROM tool_calls WHERE origin = 'mcp' AND method = 'tools/call'",
        )
    };
    let snapshot_events = count(&source, "SELECT COUNT(*) FROM snapshot_events") as usize;

    let net = replay_net_events(&source, &writer);
    let model = replay_model_calls(&source, &writer);
    let mcp = replay_mcp_calls(&source, &writer);
    let files = replay_file_events(&source, &writer);
    writer.shutdown_blocking();

    // Checked before anything else opens the file: only the ledger is
    // committed, so everything has to be in it and not in a WAL beside it.
    // (A later read-only connection recreates an empty one.)
    assert!(
        !rebuilt.with_extension("db-wal").exists(),
        "shutdown must checkpoint before the file is copied"
    );

    let rebuilt_conn = Connection::open_with_flags(&rebuilt, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let rails = [
        Rail {
            name: "net_events",
            read: net,
            expected: net as i64,
            persisted: count(&rebuilt_conn, "SELECT COUNT(*) FROM net_events"),
            declared_drop: None,
        },
        Rail {
            name: "model_calls",
            read: model,
            expected: model as i64,
            persisted: count(&rebuilt_conn, "SELECT COUNT(*) FROM model_calls"),
            declared_drop: None,
        },
        Rail {
            name: "tool_calls (native)",
            read: native_tool_calls as usize,
            expected: native_tool_calls,
            persisted: count(&rebuilt_conn, "SELECT COUNT(*) FROM tool_calls WHERE origin != 'mcp'"),
            declared_drop: None,
        },
        Rail {
            name: "tool_responses",
            read: tool_responses as usize,
            expected: tool_responses,
            persisted: count(&rebuilt_conn, "SELECT COUNT(*) FROM tool_responses"),
            declared_drop: None,
        },
        Rail {
            name: "mcp",
            read: mcp,
            expected: mcp_tool_calls,
            persisted: count(&rebuilt_conn, "SELECT COUNT(*) FROM tool_calls WHERE origin = 'mcp'"),
            declared_drop: Some("the ledger records tool invocations, not every protocol frame"),
        },
        Rail {
            name: "fs_events",
            read: files,
            expected: files as i64,
            persisted: count(&rebuilt_conn, "SELECT COUNT(*) FROM fs_events"),
            declared_drop: None,
        },
        Rail {
            name: "snapshot_events",
            read: snapshot_events,
            expected: 0,
            persisted: 0,
            declared_drop: Some("the current schema has no such table"),
        },
    ];

    for rail in &rails {
        println!("{}", rail.report());
    }
    let problems = rails.iter().flat_map(Rail::problems).collect::<Vec<_>>();
    assert!(problems.is_empty(), "{}", problems.join("\n"));

    // Bodies are a rail too, and the one the replay cannot yet carry: it reads
    // the ledger's display excerpts, and the archive holds the full bytes. On
    // the legacy fixture that is lossless, because the excerpts were all the
    // content there was. On a fixture this replay already produced it is not:
    // a second run would write the excerpts back as the bodies and quietly
    // shorten them. Refuse rather than degrade.
    let source_body_bytes = count(&source, "SELECT COALESCE(SUM(original_bytes), 0) FROM event_body_blobs");
    let rebuilt_body_bytes = count(
        &rebuilt_conn,
        "SELECT COALESCE(SUM(original_bytes), 0) FROM event_body_blobs",
    );
    println!("bodies: {source_body_bytes} source bytes -> {rebuilt_body_bytes} rebuilt bytes");
    assert!(
        rebuilt_body_bytes >= source_body_bytes,
        "the replay would lose {} bytes of recorded body content. It reads the ledger's \
         preview columns, so it can only regenerate a fixture whose bodies are still in \
         them. To regenerate this one, teach the replay to read the archive \
         (DbHandle::read_body) and set request_body_full/response_body_full from it.",
        source_body_bytes - rebuilt_body_bytes
    );
    assert!(
        net > 0 && model > 0 && files > 0,
        "the fixture must stay a real session"
    );

    // The old fixture kept request and response text in the ledger's own
    // preview columns. The current writer puts bodies in the block archive and
    // keeps only the index, so replaying that same content produces a
    // `session.bodies` beside the ledger -- and an index pointing into an
    // archive nobody committed would be worse than no fixture at all.
    let indexed_bodies = count(&rebuilt_conn, "SELECT COUNT(*) FROM event_body_blobs");
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
