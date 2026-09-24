//! The stop rollup into main.db, and the migration that carries usage rows.

use std::path::Path;

use super::*;

/// A real ledger, written through the writer so it carries its snapshot.
fn written_ledger(path: &Path, ops: Vec<crate::writer::WriteOp>) {
    let writer = crate::writer::DbWriter::open(path, 16).unwrap();
    for op in ops {
        writer.write_blocking(op);
    }
    writer.shutdown_blocking();
}

#[test]
fn update_session_rollup_from_session_db_copies_the_snapshot_by_id() {
    use crate::counters::fixtures::*;
    let dir = tempfile::tempdir().unwrap();
    let main_path = dir.path().join("main.db");
    let session_path = dir.path().join("session.db");
    let idx = SessionIndex::open(&main_path).unwrap();
    let id = "5134d89f-090d-4f88-8d62-7bc5e9231ed8";
    idx.create_session(&sample_record(id, "running")).unwrap();
    written_ledger(
        &session_path,
        vec![
            net("allowed", 1, 1),
            net("denied", 1, 1),
            net("error", 1, 1),
            model("anthropic", Some("claude"), 10, 20, 0.25, &[("bash", "native")]),
            model("anthropic", None, 0, 3, 0.05, &[("srv__x", "mcp_proxy")]),
            mcp("tools/call", "github", "search", 7),
            mcp("tools/call", "linear", "search", 3),
            // Three changes and one overflow marker, which is not one of them.
            file("created", "/a"),
            file("modified", "/a"),
            file("deleted", "/a"),
            file("overflow", ""),
            exec(1),
            audit("/bin/ls", 1_700_000_000.0),
            audit("/bin/ls", 1_700_000_001.0),
        ],
    );

    idx.update_session_rollup_from_session_db(id, "stopped", Some("2026-06-26T12:00:00Z"), &session_path)
        .unwrap();

    let record = &idx.recent(1).unwrap()[0];
    assert_eq!(record.id, id);
    assert_eq!(record.status, "stopped");
    assert_eq!(record.stopped_at.as_deref(), Some("2026-06-26T12:00:00Z"));
    assert_eq!(
        (record.total_requests, record.allowed_requests, record.denied_requests),
        (3, 1, 1)
    );
    assert_eq!((record.total_input_tokens, record.total_output_tokens), (10, 23));
    assert!((record.total_estimated_cost - 0.30).abs() < 1e-9);
    // bash and two MCP calls; the mcp_proxy mention is the model naming a call.
    assert_eq!(record.total_tool_calls, 3);
    assert_eq!(record.total_file_events, 3);
    assert_eq!((record.exec_count, record.audit_event_count), (1, 2));

    let providers: Vec<(String, i64, i64)> = idx
        .conn
        .prepare("SELECT provider, call_count, output_tokens FROM ai_usage WHERE session_id = ?1")
        .unwrap()
        .query_map([id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(providers, vec![("anthropic".to_string(), 2, 23)]);
    let tool_calls: i64 = idx
        .conn
        .query_row(
            "SELECT SUM(call_count) FROM tool_usage WHERE session_id = ?1",
            [id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(tool_calls, 3);
    // Two servers exposing a tool of the same name are two rows.
    assert_eq!(
        usage_rows(
            &idx,
            "SELECT server_name || '/' || tool_name, call_count FROM mcp_usage ORDER BY server_name"
        ),
        vec![("github/search".to_string(), 1), ("linear/search".to_string(), 1)]
    );
}

#[test]
fn update_session_rollup_from_session_db_fails_when_id_is_missing() {
    let dir = tempfile::tempdir().unwrap();
    let session_path = dir.path().join("session.db");
    let idx = SessionIndex::open_in_memory().unwrap();
    written_ledger(&session_path, Vec::new());

    let result =
        idx.update_session_rollup_from_session_db("co-work1", "stopped", Some("2026-06-26T12:00:00Z"), &session_path);

    assert!(matches!(result, Err(rusqlite::Error::QueryReturnedNoRows)));
}

#[test]
fn update_session_rollup_refuses_a_ledger_without_a_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let session_path = dir.path().join("session.db");
    let idx = SessionIndex::open_in_memory().unwrap();
    idx.create_session(&sample_record("co-work1", "running")).unwrap();
    written_ledger(&session_path, Vec::new());
    Connection::open(&session_path)
        .unwrap()
        .execute("DELETE FROM ledger_counters", [])
        .unwrap();

    let error = idx
        .update_session_rollup_from_session_db("co-work1", "stopped", None, &session_path)
        .unwrap_err();
    assert!(error.to_string().contains("ledger_counters"), "{error}");
    assert_eq!(
        idx.recent(1).unwrap()[0].status,
        "running",
        "a failed rollup records nothing"
    );
}

/// v8 keyed `mcp_usage` by tool alone, so two servers exposing `search`
/// collided. v9 keys it by server too and keeps what v8 recorded.
#[test]
fn v8_mcp_usage_is_rekeyed_by_server_and_keeps_its_rows() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(&SESSION_SCHEMA.replace(
        "PRIMARY KEY (session_id, server_name, tool_name)",
        "PRIMARY KEY (session_id, tool_name)",
    ))
    .unwrap();
    conn.execute("INSERT INTO mcp_usage VALUES ('s1', 'search', 'github', 4, 100, 9)", [])
        .unwrap();
    conn.pragma_update(None, "user_version", 8u32).unwrap();

    SessionIndex::ensure_schema(&conn).unwrap();

    let version: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
    assert_eq!(version, SCHEMA_VERSION);
    conn.execute("INSERT INTO mcp_usage VALUES ('s1', 'search', 'linear', 1, 10, 2)", [])
        .expect("a second server's tool of the same name is its own row");
    let carried: (i64, i64) = conn
        .query_row(
            "SELECT call_count, total_bytes FROM mcp_usage WHERE server_name = 'github'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(carried, (4, 100));
}
