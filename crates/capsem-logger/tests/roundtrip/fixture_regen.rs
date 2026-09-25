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
//! produces a selected generation under `test.bodies` beside the ledger; the
//! database, generation and fresh archive lock are committed together.
//!
//! Regeneration is byte-reproducible: two runs over the same source produce
//! the same `test.db` and selected generation, down to the byte. Three
//! values used to stand in the way, and each is now derived from what was
//! recorded rather than from the run. `event_body_blobs.created_at` and
//! `body_blocks.sealed_at` were the wall clock; the replay pins them
//! (`pin_replay_clock`). The `event_id` of a tool call and a tool response
//! were minted per run; `ToolCallEntry` and `ToolResponseEntry` carry one and
//! the replay passes the source id through. `model_items.event_id` was minted
//! per run too -- those rows are derived from a `ModelCall`, not replayed from
//! a source row -- and is now derived from the item's own identity, the same
//! `(trace_id, kind, content_hash, call_id)` the table's `UNIQUE` uses.
//!
//! That is what the digest in `fixture_ownership.toml` is for. It records
//! custody either way, but only a reproducible regenerator lets a reviewer run
//! the tool and diff instead of trusting a row comparison written by whoever
//! changed the fixture -- which is custody without correctness, and the shape
//! of failure this rail exists to refuse.
//!
//! Bodies are read from the source archive, not from the ledger's own columns,
//! and that is what makes the replay repeatable. Those columns are display
//! excerpts capped at `PREVIEW_BYTES`; sourcing bodies from them is lossless
//! exactly once, on a pre-archive fixture whose excerpts were all the content
//! there was, and silently shortens a 42 KB request to 2 KB on every run after
//! that. A ledger with no archive still falls back to the columns, which is the
//! pre-archive case. Either way the run compares body bytes in against body
//! bytes out and refuses rather than degrade, so the guard stands whether or
//! not the sourcing is right.

use super::*;

#[path = "fixture_regen/lift.rs"]
mod lift;
#[cfg(test)]
#[path = "fixture_regen/tests.rs"]
mod tests;
use lift::lift_v3_source;

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, UNIX_EPOCH};

use rusqlite::Connection;

const FIXTURE_ARCHIVE_ID: [u8; 16] = [0x22, 0x70, 0, 0, 0, 0, 0x40, 0, 0x80, 0, 0, 0, 0, 0, 0, 0];
const FIXTURE_GENERATION_ID: [u8; 16] = [0x22, 0x70, 0, 0, 0, 0, 0x40, 0, 0x80, 0, 0, 0, 0, 0, 0, 1];

fn fixture_generation_id() -> capsem_archive::GenerationId {
    capsem_archive::GenerationId::from_bytes(FIXTURE_GENERATION_ID).unwrap()
}

fn archive_lock_path(db_path: &std::path::Path) -> std::path::PathBuf {
    let mut name = db_path.file_name().unwrap().to_os_string();
    name.push("-archive.lock");
    db_path.with_file_name(name)
}

fn pin_fixture_archive_identity(db_path: &std::path::Path) {
    use std::io::{Seek as _, SeekFrom, Write as _};

    let conn = Connection::open(db_path).unwrap();
    let old_generation_bytes: Vec<u8> = conn
        .query_row(
            "SELECT generation_id FROM archive_state WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let old_generation = capsem_archive::GenerationId::from_bytes(old_generation_bytes.try_into().unwrap()).unwrap();
    let directory = db_path.with_extension("bodies");
    let old_generation = directory.join(old_generation.file_name());
    let archive_id = capsem_archive::ArchiveId::from_bytes(FIXTURE_ARCHIVE_ID).unwrap();
    let generation_id = fixture_generation_id();
    let generation = directory.join(generation_id.file_name());
    std::fs::rename(&old_generation, &generation).expect("pin fixture generation name");
    let mut file = std::fs::OpenOptions::new().write(true).open(&generation).unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(&capsem_archive::format::encode_file_header(archive_id, generation_id))
        .unwrap();
    file.sync_all().unwrap();
    conn.execute(
        "UPDATE archive_state SET archive_id = ?1, generation_id = ?2 WHERE singleton = 1",
        rusqlite::params![&archive_id.as_bytes()[..], &generation_id.as_bytes()[..]],
    )
    .unwrap();
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
}

fn replace_directory(source: &std::path::Path, destination: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt as _;

    if destination.is_dir() {
        std::fs::remove_dir_all(destination).unwrap();
    } else if destination.exists() {
        std::fs::remove_file(destination).unwrap();
    }
    std::fs::create_dir(destination).unwrap();
    std::fs::set_permissions(destination, std::fs::Permissions::from_mode(0o700)).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        assert!(entry.file_type().unwrap().is_file(), "fixture archive is flat");
        std::fs::copy(entry.path(), destination.join(entry.file_name())).unwrap();
    }
}

fn remove_file_or_directory(path: &std::path::Path) {
    if path.is_dir() {
        std::fs::remove_dir_all(path).unwrap();
    } else if path.exists() {
        std::fs::remove_file(path).unwrap();
    }
}

/// The instant the replay's archive index is stamped with, in nanoseconds
/// since the epoch.
///
/// `event_body_blobs.created_at` and `body_blocks.sealed_at` are the only
/// values in a rebuilt ledger that are not derived from what was recorded, so
/// reading them from the wall clock is what made a rerun differ from the run
/// before it. The replay pins them instead, to an instant it reads out of the
/// source fixture -- so the same source always produces the same stamps, and
/// the stamps still say something true about the session rather than 1970.
static REPLAY_INSTANT_NANOS: AtomicU64 = AtomicU64::new(0);

fn replay_clock() -> SystemTime {
    UNIX_EPOCH + Duration::from_nanos(REPLAY_INSTANT_NANOS.load(Ordering::Relaxed))
}

/// Pin the replay clock to the latest instant the source ledger recorded.
fn pin_replay_clock(source: &Connection) {
    let latest: Option<String> = source
        .query_row("SELECT MAX(timestamp) FROM net_events", [], |row| row.get(0))
        .unwrap_or(None);
    let instant = latest
        .and_then(|value| humantime::parse_rfc3339(&value).ok())
        .unwrap_or(UNIX_EPOCH);
    let nanos = instant.duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos() as u64;
    REPLAY_INSTANT_NANOS.store(nanos, Ordering::Relaxed);
}

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

/// The recorded bodies of a source ledger, by the row that owns each one.
///
/// Keyed `(source_table, event_id, direction)`, which is the index's own unique
/// key, so a lookup cannot pick up another row's body.
type SourceBodies = BTreeMap<(String, String, String), Vec<u8>>;

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime for the archive reads")
        .block_on(future)
}

/// Every body the source ledger archived, read back through the DB handle.
///
/// This is what makes the replay idempotent. The ledger's own columns are
/// display excerpts capped at `PREVIEW_BYTES`; the bodies live in the archive.
/// A replay sourcing bodies from the columns is lossless exactly once -- on a
/// pre-archive fixture, whose excerpts were all the content there was -- and
/// silently truncates every run after that, which is how a 42 KB request
/// becomes 2 KB.
///
/// A v2 fixture gets a temporary v3 header and shifted block offsets only for
/// this deliberate regeneration. Production still refuses v2; the fixture
/// owner can use the current reader because the block and segment layout did
/// not change between v2 and v3.
fn archived_v2_bodies(db_path: &std::path::Path, conn: &Connection) -> SourceBodies {
    use std::io::Write as _;

    let legacy = std::fs::read(db_path.with_extension("bodies")).expect("read the v2 fixture archive");
    assert!(legacy.len() >= 16, "v2 fixture archive has a complete header");
    assert_eq!(&legacy[..8], capsem_archive::format::FILE_MAGIC);
    assert_eq!(u16::from_le_bytes(legacy[8..10].try_into().unwrap()), 2);
    assert!(legacy[10..16].iter().all(|byte| *byte == 0));

    let staging = tempfile::tempdir().unwrap();
    let upgraded = staging.path().join("fixture-v3.cbl");
    let archive_id = capsem_archive::ArchiveId::from_bytes(FIXTURE_ARCHIVE_ID).unwrap();
    let generation_id = fixture_generation_id();
    let mut file = std::fs::File::create(&upgraded).unwrap();
    file.write_all(&capsem_archive::format::encode_file_header(archive_id, generation_id))
        .unwrap();
    file.write_all(&legacy[16..]).unwrap();
    file.sync_all().unwrap();
    drop(file);

    let reader = capsem_archive::BodyLogReader::open(&upgraded).unwrap();
    let shift = u64::try_from(capsem_archive::FILE_HEADER_BYTES - 16).unwrap();
    let mut statement = conn
        .prepare(
            "SELECT b.source_table, b.event_id, b.direction, b.body_hash,
                    b.block_offset, b.body_offset, b.body_len,
                    blocks.disk_len, blocks.raw_len
             FROM event_body_blobs AS b
             JOIN body_blocks AS blocks ON blocks.block_offset = b.block_offset
             ORDER BY b.block_offset, b.body_offset",
        )
        .unwrap();
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, u64>(4)?,
                row.get::<_, u32>(5)?,
                row.get::<_, u32>(6)?,
                row.get::<_, u64>(7)?,
                row.get::<_, u32>(8)?,
            ))
        })
        .unwrap();
    let mut bodies = SourceBodies::new();
    for row in rows {
        let (source_table, event_id, direction, expected_hash, block_offset, offset, len, disk_len, raw_len) =
            row.unwrap();
        let bytes = reader
            .read_bounded(
                capsem_archive::BodyRef {
                    block_offset: block_offset + shift,
                    offset,
                    len,
                },
                capsem_archive::BlockExtent { disk_len, raw_len },
            )
            .unwrap();
        assert_eq!(format!("blake3:{}", blake3::hash(&bytes).to_hex()), expected_hash);
        bodies.insert((source_table, event_id, direction), bytes);
    }
    bodies
}

fn archived_bodies(db_path: &std::path::Path, conn: &Connection) -> SourceBodies {
    if !table_exists(conn, "event_body_blobs") {
        return SourceBodies::new();
    }
    if !table_exists(conn, "archive_state") {
        return archived_v2_bodies(db_path, conn);
    }
    let mut stmt = conn
        .prepare("SELECT DISTINCT event_id FROM event_body_blobs ORDER BY event_id")
        .expect("read the body index");
    let event_ids: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .expect("read the body index")
        .map(Result::unwrap)
        .collect();
    if event_ids.is_empty() {
        return SourceBodies::new();
    }

    // A v3 source has the same body archive a v4 ledger has; v4 only added
    // the counter snapshot. Reading its bodies goes through a scratch copy
    // with the marker lifted, which the product never does: nothing upgrades
    // a ledger in place, and the replay rebuilds the counters from the rows.
    let lifted = lift_v3_source(db_path, conn);
    let db_path = lifted
        .as_ref()
        .map_or(db_path.to_path_buf(), |dir| dir.path().join("test.db"));
    let db = capsem_logger::DbHandle::open_external_reader(&db_path).expect("open the source ledger's archive");
    let mut bodies = SourceBodies::new();
    for event_id in event_ids {
        // One query per event returns every direction it stored, and the
        // handle verifies each body against the row that named it.
        for body in block_on(db.read_bodies(&event_id)).expect("read the archived bodies") {
            bodies.insert(
                (body.source_table, body.event_id, body.direction.as_str().to_string()),
                body.bytes,
            );
        }
    }
    bodies
}

/// The archived body for one row, or the column excerpt when there is none.
fn body_of(
    bodies: &SourceBodies,
    source_table: &str,
    event_id: Option<&str>,
    direction: &str,
    column: Option<String>,
) -> Option<Vec<u8>> {
    let archived = event_id
        .and_then(|event_id| bodies.get(&(source_table.to_string(), event_id.to_string(), direction.to_string())));
    archived.cloned().or_else(|| column.map(String::into_bytes))
}

/// The same lookup for a row whose field is text rather than bytes.
///
/// An archived body that is not valid UTF-8 cannot be carried by a `String`
/// field, so it falls back to the column and the byte accounting at the end of
/// the replay reports the difference rather than hiding it.
fn text_of(
    bodies: &SourceBodies,
    source_table: &str,
    event_id: Option<&str>,
    direction: &str,
    column: Option<String>,
) -> Option<String> {
    body_of(bodies, source_table, event_id, direction, None)
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .or(column)
}

fn decision_of(value: &str) -> Decision {
    match value {
        "denied" => Decision::Denied,
        "error" => Decision::Error,
        "redirected" => Decision::Redirected,
        _ => Decision::Allowed,
    }
}

fn replay_net_events(conn: &Connection, writer: &DbWriter, bodies: &SourceBodies) -> usize {
    let mut stmt = conn.prepare("SELECT * FROM net_events ORDER BY id").unwrap();
    let events = stmt
        .query_map([], |row| {
            let event_id: Option<String> = opt(row, "event_id");
            Ok(NetEvent {
                event_id: event_id.clone(),
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
                request_body: body_of(
                    bodies,
                    "net_events",
                    event_id.as_deref(),
                    "request",
                    opt(row, "request_body_preview"),
                ),
                response_body: body_of(
                    bodies,
                    "net_events",
                    event_id.as_deref(),
                    "response",
                    opt(row, "response_body_preview"),
                ),
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
            event_id: opt(row, "event_id"),
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

fn tool_responses_for(conn: &Connection, model_call_id: i64, bodies: &SourceBodies) -> Vec<ToolResponseEntry> {
    let mut stmt = conn
        .prepare("SELECT * FROM tool_responses WHERE model_call_id = ?1 ORDER BY id")
        .unwrap();
    stmt.query_map([model_call_id], |row| {
        Ok(ToolResponseEntry {
            event_id: opt(row, "event_id"),
            call_id: opt(row, "call_id").unwrap_or_default(),
            content_preview: text_of(
                bodies,
                "tool_responses",
                opt::<String>(row, "event_id").as_deref(),
                "response",
                opt(row, "content_preview"),
            ),
            is_error: opt::<i64>(row, "is_error").unwrap_or_default() != 0,
            trace_id: opt(row, "trace_id"),
            credential_ref: opt(row, "credential_ref"),
        })
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

fn replay_model_calls(conn: &Connection, writer: &DbWriter, bodies: &SourceBodies) -> usize {
    let mut stmt = conn.prepare("SELECT * FROM model_calls ORDER BY id").unwrap();
    let calls = stmt
        .query_map([], |row| {
            let id: i64 = row.get("id")?;
            let event_id: Option<String> = opt(row, "event_id");
            Ok((
                id,
                ModelCall {
                    event_id: event_id.clone(),
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
                    request_body: body_of(
                        bodies,
                        "model_calls",
                        event_id.as_deref(),
                        "request",
                        opt(row, "request_body_preview"),
                    ),
                    message_id: opt(row, "message_id"),
                    status_code: opt::<i64>(row, "status_code").map(|v| v as u16),
                    text_content: opt(row, "text_content"),
                    thinking_content: opt(row, "thinking_content"),
                    response_body: body_of(
                        bodies,
                        "model_calls",
                        event_id.as_deref(),
                        "response",
                        opt(row, "response_body_full"),
                    ),
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
        call.tool_responses = tool_responses_for(conn, id, bodies);
        writer.write_blocking(WriteOp::ModelCall(call));
    }
    count
}

fn replay_mcp_calls(conn: &Connection, writer: &DbWriter, bodies: &SourceBodies) -> usize {
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
            let event_id: Option<String> = opt(row, "event_id");
            Ok(McpCall {
                event_id: event_id.clone(),
                timestamp: at(row, "timestamp"),
                server_name: opt(row, "server_name").unwrap_or_default(),
                method: opt(row, "method").unwrap_or_default(),
                tool_name: opt(row, "tool_name"),
                request_id: opt(row, "request_id"),
                request_preview: text_of(
                    bodies,
                    "tool_calls",
                    event_id.as_deref(),
                    "request",
                    opt(row, "request_preview").or_else(|| opt(row, "arguments")),
                ),
                response_preview: text_of(
                    bodies,
                    "tool_calls",
                    event_id.as_deref(),
                    "response",
                    opt(row, "response_preview"),
                ),
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

/// A count that fails loudly.
///
/// This used to swallow every error into 0, which made "the table is not there,
/// as expected" and "the query is broken" the same answer. A renamed
/// `event_body_blobs` would then have read 0 source body bytes and waved a
/// lossy rerun straight through the guard below.
fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get(0))
        .unwrap_or_else(|error| panic!("counting rows failed: {sql}: {error}"))
}

/// The same count for a table that may legitimately be absent from the shape
/// being read. Absence answers 0; anything else still fails.
fn count_if_table(conn: &Connection, table: &str, sql: &str) -> i64 {
    if table_exists(conn, table) {
        count(conn, sql)
    } else {
        0
    }
}

#[test]
#[ignore = "rewrites the checked-in fixture; run deliberately, then commit the binary and its sha256"]
fn regenerate_session_fixture() {
    let fixture = fixture_path();
    let source = Connection::open_with_flags(&fixture, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .expect("the existing fixture must be readable");

    let staging = tempfile::tempdir().unwrap();
    let rebuilt = staging.path().join("session.db");
    pin_replay_clock(&source);
    let writer = DbWriter::open_with_clock(&rebuilt, 256, replay_clock).unwrap();

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
    let snapshot_events = count_if_table(&source, "snapshot_events", "SELECT COUNT(*) FROM snapshot_events") as usize;

    // Bodies come from the archive, not from the display columns beside them.
    let bodies = archived_bodies(&fixture, &source);
    // A flush after each rail is a segment of the open block, as a live
    // session's timed flushes are, so the fixture's archive has the shape the
    // product writes -- several segments, the last one closing the block --
    // and every reader of it is exercised across segment boundaries. Explicit
    // barriers rather than the timer, so the layout is the same on every run.
    let net = replay_net_events(&source, &writer, &bodies);
    block_on(writer.flush());
    let model = replay_model_calls(&source, &writer, &bodies);
    block_on(writer.flush());
    let mcp = replay_mcp_calls(&source, &writer, &bodies);
    block_on(writer.flush());
    let files = replay_file_events(&source, &writer);
    drop(bodies);
    writer.shutdown_blocking();
    pin_fixture_archive_identity(&rebuilt);

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
    let source_body_bytes = count_if_table(
        &source,
        "event_body_blobs",
        "SELECT COALESCE(SUM(original_bytes), 0) FROM event_body_blobs",
    );
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
         (DbHandle::read_body) and set request_body/response_body from it.",
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
    let rebuilt_lock = archive_lock_path(&rebuilt);
    let fixture_lock = archive_lock_path(&fixture);
    println!("indexed bodies: {indexed_bodies}");
    // The same ordering the writer itself keeps: bytes before the index that
    // names them. If the second copy fails, the pair left behind is a stale
    // archive under an old index, not an index pointing into bytes that are not
    // there. Removal runs the other way for the same reason -- the index goes
    // first, so nothing is left naming an archive that is gone.
    if indexed_bodies > 0 {
        replace_directory(&rebuilt_bodies, &fixture_bodies);
        std::fs::copy(&rebuilt_lock, &fixture_lock).expect("write the regenerated archive lock");
        std::fs::copy(&rebuilt, &fixture).expect("write the regenerated fixture");
    } else {
        std::fs::copy(&rebuilt, &fixture).expect("write the regenerated fixture");
        remove_file_or_directory(&fixture_bodies);
        remove_file_or_directory(&fixture_lock);
    }

    let reader = DbReader::open(&fixture).unwrap();
    reader
        .ready()
        .expect("the regenerated fixture must be a current-shape ledger");
    drop(reader);

    // Opening the fixture leaves a WAL and an shm index beside it. Only the two
    // committed files belong in the tree, so a run that left those behind would
    // put two untracked SQLite sidecars there for someone to `git add -A` by
    // accident. The WAL is checked empty before either is removed -- a WAL with
    // content would mean the ledger is not entirely in the file being committed.
    // (The shm is a fixed-size shared-memory index and carries no ledger bytes,
    // so its size says nothing.)
    let wal = fixture.with_extension("db-wal");
    if let Ok(metadata) = std::fs::metadata(&wal) {
        assert_eq!(
            metadata.len(),
            0,
            "{} holds {} bytes the committed fixture would not carry",
            wal.display(),
            metadata.len()
        );
    }
    for extension in ["db-wal", "db-shm"] {
        let sidecar = fixture.with_extension(extension);
        if sidecar.exists() {
            std::fs::remove_file(&sidecar).unwrap();
        }
    }

    assert_recorded_digests(&[
        fixture,
        fixture_bodies.join(fixture_generation_id().file_name()),
        fixture_lock,
    ]);
}

/// Fail unless what was just written is what `fixture_ownership.toml` records.
///
/// `tests/citadel/test_fixture_ownership.py` compares the same digests, so
/// a stale entry is caught either way. What it cannot do is catch it *here*,
/// at the moment the bytes change, with the digests to record printed in the
/// failure. Without that, forgetting the toml is a green regeneration followed
/// by a red citadel run in a different suite, and the person who has to
/// connect the two is whoever runs the gate next rather than whoever moved
/// the fixture.
///
/// It is also the standing proof that regeneration is reproducible: a run
/// against an unchanged tree rewrites the fixture and must arrive back at the
/// digests already on record.
/// The `sha256` recorded for the entry whose path ends in `name`.
///
/// `fixture_ownership.toml` lists a fixture's `source`/`path` and then its
/// `sha256`, so the digest belonging to a file is the first one after the line
/// that names it.
fn recorded_digest(ownership: &str, name: &str) -> Option<String> {
    let mut seen_entry = false;
    for line in ownership.lines() {
        let line = line.trim();
        if line.starts_with("source =") || line.starts_with("path =") {
            seen_entry = line.contains(name);
        } else if seen_entry {
            if let Some(value) = line.strip_prefix("sha256 = ") {
                return Some(value.trim_matches('"').to_string());
            }
        }
    }
    None
}

fn assert_recorded_digests(paths: &[std::path::PathBuf]) {
    use sha2::{Digest, Sha256};

    let ownership = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/citadel/fixture_ownership.toml")
        .canonicalize()
        .expect("fixture_ownership.toml must exist beside the fixture it records");
    let recorded = std::fs::read_to_string(&ownership).expect("read fixture_ownership.toml");

    let mut stale = Vec::new();
    for path in paths {
        let name = path.file_name().expect("fixture name").to_string_lossy().into_owned();
        assert!(
            path.exists(),
            "{name} was not written; a fixture the regenerator skips is one nobody is checking"
        );
        let digest = format!("{:x}", Sha256::digest(std::fs::read(path).expect("read fixture")));
        // Keyed, not a substring scan of the whole file: both fixtures are
        // recorded in it, so `contains` would let test.db pass on test.bodies'
        // digest -- the two are never equal, but the check would be saying
        // "some fixture has these bytes", which is not the question.
        if recorded_digest(&recorded, &name).as_deref() != Some(digest.as_str()) {
            stale.push(format!("  {name}: sha256 = \"{digest}\""));
        }
    }
    assert!(
        stale.is_empty(),
        "the regenerated fixture does not match the digests in {}.\n\
         Record these in the same commit as the new bytes:\n{}",
        ownership.display(),
        stale.join("\n")
    );
}
