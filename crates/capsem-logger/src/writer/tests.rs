//! Tests for `writer` (extracted from inline `mod tests`).

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
fn net_event_stores_bounded_body_blobs_and_small_previews() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("body-blobs.db");
    let event_id = "abc123def456".to_string();
    let trace_id = "trace-body-blob".to_string();
    let request_body = format!("{{\"prompt\":\"{}\"}}", "r".repeat(MAX_FIELD_BYTES + 1024));
    let request_preview = "{\"prompt\":\"short\"}".to_string();
    let response_body = format!(
        "event: message\ndata: {}\n\n",
        "s".repeat(MAX_BODY_BLOB_BYTES + 128)
    );
    let response_preview = "event: message\ndata: short\n\n".to_string();
    let response_hash = blake3_bytes_ref(response_body.as_bytes());

    {
        let writer = DbWriter::open(&db_path, 64).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(async {
            writer
                .write(WriteOp::NetEvent(crate::events::NetEvent {
                    event_id: Some(event_id.clone()),
                    timestamp: std::time::SystemTime::now(),
                    domain: "daily-cloudcode-pa.googleapis.com".into(),
                    port: 443,
                    decision: crate::events::Decision::Allowed,
                    process_name: Some("agy".into()),
                    pid: Some(1234),
                    method: Some("POST".into()),
                    path: Some("/v1internal:streamGenerateContent".into()),
                    query: None,
                    status_code: Some(200),
                    bytes_sent: request_body.len() as u64,
                    bytes_received: response_body.len() as u64,
                    duration_ms: 42,
                    matched_rule: Some("profiles.rules.ai_google_http_googleapis".into()),
                    request_headers: Some("content-type: application/json".into()),
                    response_headers: Some("content-type: text/event-stream".into()),
                    request_body_preview: Some(request_preview.clone()),
                    response_body_preview: Some(response_preview.clone()),
                    request_body_full: Some(request_body.clone()),
                    response_body_full: Some(response_body.clone()),
                    conn_type: Some("https-mitm".into()),
                    policy_mode: None,
                    policy_action: Some("allow".into()),
                    policy_rule: Some("profiles.rules.ai_google_http_googleapis".into()),
                    policy_reason: None,
                    trace_id: Some(trace_id.clone()),
                    credential_ref: None,
                }))
                .await;
        });
    }

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let (stored_request_preview, stored_response_preview): (String, String) = conn
        .query_row(
            "SELECT request_body_preview, response_body_preview FROM net_events WHERE event_id = ?1",
            [&event_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(stored_request_preview, request_preview);
    assert_eq!(stored_response_preview, response_preview);

    struct StoredBlob {
        direction: String,
        event_type: String,
        content_type: String,
        original_bytes: i64,
        stored_bytes: i64,
        truncated: i64,
        body_hash: String,
        body: Vec<u8>,
        trace_id: String,
    }

    let blobs: Vec<StoredBlob> = conn
        .prepare(
            "SELECT direction, event_type, content_type, original_bytes, stored_bytes,
                    truncated, body_hash, body, trace_id
             FROM event_body_blobs
             WHERE event_id = ?1
             ORDER BY direction",
        )
        .unwrap()
        .query_map([&event_id], |row| {
            Ok(StoredBlob {
                direction: row.get(0)?,
                event_type: row.get(1)?,
                content_type: row.get(2)?,
                original_bytes: row.get(3)?,
                stored_bytes: row.get(4)?,
                truncated: row.get(5)?,
                body_hash: row.get(6)?,
                body: row.get(7)?,
                trace_id: row.get(8)?,
            })
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(blobs.len(), 2);

    let request = blobs
        .iter()
        .find(|blob| blob.direction == "request")
        .unwrap();
    assert_eq!(request.event_type, "http.request");
    assert_eq!(request.content_type, "application/json");
    assert_eq!(request.original_bytes, request_body.len() as i64);
    assert_eq!(request.stored_bytes, request_body.len() as i64);
    assert_eq!(request.truncated, 0);
    assert_eq!(request.body_hash, blake3_bytes_ref(request_body.as_bytes()));
    assert_eq!(request.body, request_body.as_bytes());
    assert_eq!(request.trace_id, trace_id);

    let response = blobs
        .iter()
        .find(|blob| blob.direction == "response")
        .unwrap();
    assert_eq!(response.event_type, "http.request");
    assert_eq!(response.content_type, "text/event-stream");
    assert_eq!(response.original_bytes, response_body.len() as i64);
    assert_eq!(response.stored_bytes, MAX_BODY_BLOB_BYTES as i64);
    assert_eq!(response.truncated, 1);
    assert_eq!(response.body_hash, response_hash);
    assert_eq!(response.body.len(), MAX_BODY_BLOB_BYTES);
    assert_eq!(
        &response.body,
        &response_body.as_bytes()[..MAX_BODY_BLOB_BYTES]
    );
    assert_eq!(response.trace_id, trace_id);
}

#[test]
fn db_writer_checkpoints_wal_on_drop() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.db");

    // Write some events, then drop the writer.
    {
        let writer = DbWriter::open(&db_path, 64).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(async {
            writer
                .write(WriteOp::FileEvent(crate::events::FileEvent {
                    event_id: None,
                    timestamp: std::time::SystemTime::now(),
                    action: crate::events::FileAction::Created,
                    path: "/tmp/test".to_string(),
                    size: Some(42),
                    trace_id: None,
                    credential_ref: None,
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
fn writer_generates_twelve_hex_event_id_for_primary_events() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("event-id.db");

    {
        let writer = DbWriter::open(&db_path, 64).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(async {
            writer
                .write(WriteOp::FileEvent(crate::events::FileEvent {
                    event_id: None,
                    timestamp: std::time::SystemTime::now(),
                    action: crate::events::FileAction::Created,
                    path: "/tmp/event-id".to_string(),
                    size: Some(42),
                    trace_id: None,
                    credential_ref: None,
                }))
                .await;
        });
    }

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let event_id: String = conn
        .query_row("SELECT event_id FROM fs_events LIMIT 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(event_id.len(), 12);
    assert!(event_id
        .chars()
        .all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase()));
}

#[test]
fn writer_preserves_supplied_primary_event_id() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("supplied-event-id.db");

    {
        let writer = DbWriter::open(&db_path, 64).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(async {
            writer
                .write(WriteOp::FileEvent(crate::events::FileEvent {
                    event_id: Some("abcdef123456".to_string()),
                    timestamp: std::time::SystemTime::now(),
                    action: crate::events::FileAction::Created,
                    path: "/tmp/event-id".to_string(),
                    size: Some(42),
                    trace_id: None,
                    credential_ref: None,
                }))
                .await;
        });
    }

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let event_id: String = conn
        .query_row("SELECT event_id FROM fs_events LIMIT 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(event_id, "abcdef123456");
}

#[test]
fn snapshot_fs_events_cross_reference() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("cross.db");

    {
        let writer = DbWriter::open(&db_path, 64).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(async {
            // Write some fs_events first.
            for i in 0..5 {
                writer
                    .write(WriteOp::FileEvent(crate::events::FileEvent {
                        event_id: None,
                        timestamp: std::time::SystemTime::now(),
                        action: crate::events::FileAction::Created,
                        path: format!("file_{i}.txt"),
                        size: Some(100),
                        trace_id: None,
                        credential_ref: None,
                    }))
                    .await;
            }
            for i in 5..8 {
                writer
                    .write(WriteOp::FileEvent(crate::events::FileEvent {
                        event_id: None,
                        timestamp: std::time::SystemTime::now(),
                        action: crate::events::FileAction::Modified,
                        path: format!("file_{i}.txt"),
                        size: Some(200),
                        trace_id: None,
                        credential_ref: None,
                    }))
                    .await;
            }
            writer
                .write(WriteOp::FileEvent(crate::events::FileEvent {
                    event_id: None,
                    timestamp: std::time::SystemTime::now(),
                    action: crate::events::FileAction::Deleted,
                    path: "old.txt".to_string(),
                    size: None,
                    trace_id: None,
                    credential_ref: None,
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
fn shutdown_blocking_through_arc_flushes_wal() {
    // Verifies the explicit-cleanup contract: callers holding
    // Arc<DbWriter> can drain the writer thread synchronously through
    // &self, without waiting for the last Arc clone to drop. This is
    // the path taken by capsem-process's SIGTERM handler.
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("shutdown.db");
    let writer = Arc::new(DbWriter::open(&db_path, 64).unwrap());

    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    rt.block_on(async {
        writer
            .write(WriteOp::FileEvent(crate::events::FileEvent {
                event_id: None,
                timestamp: std::time::SystemTime::now(),
                action: crate::events::FileAction::Created,
                path: "/x".into(),
                size: Some(1),
                trace_id: None,
                credential_ref: None,
            }))
            .await;
    });

    // Additional Arc clone stays alive across shutdown; the explicit
    // shutdown must not require the clone to drop first.
    let _keep = Arc::clone(&writer);
    writer.shutdown_blocking();

    let wal_path = dir.path().join("shutdown.db-wal");
    if wal_path.exists() {
        assert_eq!(
            std::fs::metadata(&wal_path).unwrap().len(),
            0,
            "WAL must be checkpointed after shutdown_blocking"
        );
    }

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM fs_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1, "durable write must survive shutdown_blocking");
}

#[test]
fn shutdown_blocking_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let writer = DbWriter::open(&dir.path().join("idemp.db"), 16).unwrap();
    writer.shutdown_blocking();
    // Second call must not panic or double-join.
    writer.shutdown_blocking();
}

#[test]
fn write_after_shutdown_is_noop() {
    let dir = tempfile::tempdir().unwrap();
    let writer = DbWriter::open(&dir.path().join("no.db"), 16).unwrap();
    writer.shutdown_blocking();
    assert!(
        !writer.try_write(WriteOp::FileEvent(crate::events::FileEvent {
            event_id: None,
            timestamp: std::time::SystemTime::now(),
            action: crate::events::FileAction::Created,
            path: "/after".into(),
            size: None,
            trace_id: None,
            credential_ref: None,
        }))
    );
}

#[tokio::test]
async fn security_rule_event_roundtrip_preserves_forensic_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("security-rule.db");
    let writer = DbWriter::open(&db_path, 64).unwrap();

    writer
        .write(WriteOp::SecurityRuleEvent(
            crate::events::SecurityRuleEvent {
                timestamp_unix_ms: 1_789_000_000_000,
                event_id: "abcdef123456".into(),
                event_type: "model.call".into(),
                rule_id: "openai_api_block".into(),
                rule_action: crate::events::SecurityRuleAction::Block,
                detection_level: crate::events::SecurityDetectionLevel::Critical,
                rule_json: r#"{"name":"openai_api_block","match":"model.provider == \"openai\""}"#
                    .into(),
                event_json:
                    r#"{"common":{"event_type":"model.call"},"model":{"provider":"openai"}}"#
                        .into(),
                trace_id: Some("trace_abc".into()),
                turn_id: Some("turn_abc".into()),
                credential_ref: Some(crate::events::credential_reference("openai", "sk-test")),
            },
        ))
        .await;
    drop(writer);

    let reader = crate::reader::DbReader::open(&db_path).unwrap();
    let events = reader.recent_security_rule_events(10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_id, "abcdef123456");
    assert_eq!(events[0].event_type, "model.call");
    assert_eq!(events[0].rule_id, "openai_api_block");
    assert_eq!(
        events[0].rule_action,
        crate::events::SecurityRuleAction::Block
    );
    assert_eq!(
        events[0].detection_level,
        crate::events::SecurityDetectionLevel::Critical
    );
    assert!(events[0].rule_json.contains("openai_api_block"));
    assert!(events[0].event_json.contains("model.call"));
    assert_eq!(events[0].trace_id.as_deref(), Some("trace_abc"));
    assert_eq!(events[0].turn_id.as_deref(), Some("turn_abc"));
    assert!(events[0]
        .credential_ref
        .as_deref()
        .is_some_and(crate::events::is_credential_reference));
}

#[tokio::test]
async fn profile_mutation_event_roundtrip_preserves_profile_ledger() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("profile-mutation.db");
    let writer = DbWriter::open(&db_path, 64).unwrap();

    writer
        .write(WriteOp::ProfileMutationEvent(
            crate::events::ProfileMutationEvent {
                timestamp_unix_ms: 1_789_000_000_000,
                mutation_id: "a1b2c3d4e5f6".into(),
                profile_id: "code".into(),
                actor: "ui".into(),
                category: "mcp".into(),
                filename: "enforcement.toml".into(),
                affected_path: "profiles/code/enforcement.toml".into(),
                target_kind: "mcp_tool".into(),
                target_key: "capsem/fetch_http".into(),
                operation: "permission".into(),
                rule_id: Some("profiles.rules.mcp_capsem_fetch_http_permission".into()),
                old_hash: format!("blake3:{}", "1".repeat(64)),
                old_size: 10,
                new_hash: format!("blake3:{}", "2".repeat(64)),
                new_size: 20,
                status: crate::events::ProfileMutationStatus::Applied,
                error: None,
                trace_id: Some("trace_profile".into()),
            },
        ))
        .await;
    drop(writer);

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let row: (
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        i64,
        String,
    ) = conn
        .query_row(
            "SELECT profile_id, actor, category, filename, target_kind, target_key,
                    rule_id, new_size, status
             FROM profile_mutation_events WHERE mutation_id = 'a1b2c3d4e5f6'",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        row,
        (
            "code".into(),
            "ui".into(),
            "mcp".into(),
            "enforcement.toml".into(),
            "mcp_tool".into(),
            "capsem/fetch_http".into(),
            "profiles.rules.mcp_capsem_fetch_http_permission".into(),
            20,
            "applied".into(),
        )
    );
}

#[test]
fn profile_mutation_schema_rejects_bad_status_and_hashes() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::schema::create_tables(&conn).unwrap();

    let bad_status = conn.execute(
        "INSERT INTO profile_mutation_events (
            timestamp_unix_ms, mutation_id, profile_id, actor, category, filename,
            affected_path, target_kind, target_key, operation,
            old_hash, old_size, new_hash, new_size, status
         )
         VALUES (1, 'a1b2c3d4e5f6', 'code', 'ui', 'mcp', 'enforcement.toml',
            'profiles/code/enforcement.toml', 'mcp_tool', 'capsem/fetch_http',
            'permission', ?1, 1, ?2, 1, 'maybe')",
        rusqlite::params![
            format!("blake3:{}", "1".repeat(64)),
            format!("blake3:{}", "2".repeat(64)),
        ],
    );
    assert!(bad_status.is_err(), "invalid mutation status must fail");

    let bad_hash = conn.execute(
        "INSERT INTO profile_mutation_events (
            timestamp_unix_ms, mutation_id, profile_id, actor, category, filename,
            affected_path, target_kind, target_key, operation,
            old_hash, old_size, new_hash, new_size, status
         )
         VALUES (1, 'a1b2c3d4e5f6', 'code', 'ui', 'mcp', 'enforcement.toml',
            'profiles/code/enforcement.toml', 'mcp_tool', 'capsem/fetch_http',
            'permission', 'sha256:nope', 1, ?1, 1, 'applied')",
        [format!("blake3:{}", "2".repeat(64))],
    );
    assert!(bad_hash.is_err(), "non-BLAKE3 profile pins must fail");
}

#[tokio::test]
async fn security_ask_event_roundtrip_preserves_lifecycle_rows() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("security-ask.db");
    let writer = DbWriter::open(&db_path, 64).unwrap();
    let pending = crate::events::SecurityAskEvent::pending(crate::events::SecurityAskPending {
        timestamp_unix_ms: 1_789_000_000_000,
        ask_id: "abcdef123456".to_string(),
        event_id: "111111abcdef".to_string(),
        event_type: "http.request".to_string(),
        rule_id: "profiles.rules.ask_openai".to_string(),
        rule_name: "ask_openai".to_string(),
        rule_json: r#"{"name":"ask_openai"}"#.to_string(),
        event_json: r#"{"http":{"host":"api.openai.com"}}"#.to_string(),
    })
    .with_trace_id("trace_ask");
    let approved = pending
        .clone()
        .with_status(crate::events::SecurityAskStatus::Approved)
        .with_resolver("tester")
        .with_reason("approved");

    writer
        .write(WriteOp::SecurityAskEvent(pending.clone()))
        .await;
    writer.write(WriteOp::SecurityAskEvent(approved)).await;
    drop(writer);

    let reader = crate::reader::DbReader::open(&db_path).unwrap();
    let rows = reader.recent_security_ask_events(10).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].status, crate::events::SecurityAskStatus::Approved);
    assert_eq!(rows[0].resolver.as_deref(), Some("tester"));
    assert_eq!(rows[1].status, crate::events::SecurityAskStatus::Pending);
    assert_eq!(rows[1].event_id, "111111abcdef");
    assert_eq!(rows[1].rule_id, "profiles.rules.ask_openai");
    let latest = reader
        .latest_security_ask_event("abcdef123456")
        .unwrap()
        .unwrap();
    assert_eq!(latest.status, crate::events::SecurityAskStatus::Approved);
}

#[tokio::test]
async fn security_decision_event_roundtrip_preserves_explicit_transition() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("security-decision.db");
    let writer = DbWriter::open(&db_path, 64).unwrap();

    writer
        .write(WriteOp::SecurityDecisionEvent(
            crate::events::SecurityDecisionEvent {
                timestamp_unix_ms: 1_789_000_000_000,
                event_id: "abcdef123456".into(),
                event_type: "file.import".into(),
                stage: crate::events::SecurityDecisionStage::Rewrite,
                actor: "dummy_pre_eicar".into(),
                rule_id: Some("profiles.rules.scan_eicar".into()),
                plugin_id: Some("dummy_pre_eicar".into()),
                previous_decision: crate::events::SecurityDecision::Allow,
                requested_decision: crate::events::SecurityDecision::Block,
                effective_decision: crate::events::SecurityDecision::Block,
                reason: Some("EICAR test seed observed".into()),
                event_json: r#"{"file":{"import":{"name":"eicar.txt"}}}"#.into(),
                trace_id: Some("trace_eicar".into()),
                turn_id: Some("turn_eicar".into()),
                credential_ref: Some(crate::events::credential_reference("github", "ghp-test")),
            },
        ))
        .await;
    drop(writer);

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let row: (
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
    ) = conn
        .query_row(
            "SELECT stage, actor, previous_decision, requested_decision,
                    effective_decision, reason, trace_id, turn_id, credential_ref
             FROM security_decision_events WHERE event_id = 'abcdef123456'",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        row,
        (
            "rewrite".into(),
            "dummy_pre_eicar".into(),
            "allow".into(),
            "block".into(),
            "block".into(),
            "EICAR test seed observed".into(),
            "trace_eicar".into(),
            "turn_eicar".into(),
            crate::events::credential_reference("github", "ghp-test"),
        )
    );
}

#[tokio::test]
async fn security_rule_stats_are_regenerated_from_session_db() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("security-rule-stats.db");
    let writer = DbWriter::open(&db_path, 64).unwrap();

    for (idx, action, level) in [
        (
            1,
            crate::events::SecurityRuleAction::Block,
            crate::events::SecurityDetectionLevel::Critical,
        ),
        (
            2,
            crate::events::SecurityRuleAction::Block,
            crate::events::SecurityDetectionLevel::Critical,
        ),
        (
            3,
            crate::events::SecurityRuleAction::Allow,
            crate::events::SecurityDetectionLevel::None,
        ),
    ] {
        writer
            .write(WriteOp::SecurityRuleEvent(
                crate::events::SecurityRuleEvent {
                    timestamp_unix_ms: 1_789_000_000_000 + idx,
                    event_id: format!("{idx:012x}"),
                    event_type: if idx == 3 {
                        "http.request".into()
                    } else {
                        "model.call".into()
                    },
                    rule_id: if idx == 3 {
                        "github_api_allow".into()
                    } else {
                        "openai_api_block".into()
                    },
                    rule_action: action,
                    detection_level: level,
                    rule_json: "{}".into(),
                    event_json: "{}".into(),
                    trace_id: None,
                    turn_id: None,
                    credential_ref: None,
                },
            ))
            .await;
    }
    drop(writer);

    let reader = crate::reader::DbReader::open(&db_path).unwrap();
    let stats = reader.security_rule_stats().unwrap();
    assert_eq!(stats.total, 3);
    assert!(stats
        .by_action
        .iter()
        .any(|entry| entry.rule_action == "block" && entry.count == 2));
    assert!(stats
        .by_event_type
        .iter()
        .any(|entry| entry.event_type == "model.call" && entry.count == 2));
    let block = stats
        .by_rule
        .iter()
        .find(|entry| entry.rule_id == "openai_api_block")
        .unwrap();
    assert_eq!(block.rule_action, "block");
    assert_eq!(block.detection_level, "critical");
    assert_eq!(block.count, 2);
    assert_eq!(block.latest_event_id, "000000000002");
}

#[test]
fn slow_checkpoint_hook_delays_shutdown() {
    // Sets CAPSEM_TEST_SLOW_CHECKPOINT_MS on the spawned writer thread
    // (env var is inherited by the thread). Asserts shutdown_blocking
    // waits for the delayed checkpoint rather than returning early --
    // which is precisely what an implicit runtime-drop path would fail
    // to guarantee under a tight SIGKILL budget.
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: std::env::set_var is unsafe on 2024 edition -- single
    // writer in this test, no concurrent readers.
    unsafe { std::env::set_var("CAPSEM_TEST_SLOW_CHECKPOINT_MS", "200") };
    let writer = DbWriter::open(&dir.path().join("slow.db"), 16).unwrap();
    let start = std::time::Instant::now();
    writer.shutdown_blocking();
    let elapsed = start.elapsed();
    unsafe { std::env::remove_var("CAPSEM_TEST_SLOW_CHECKPOINT_MS") };
    assert!(
        elapsed >= std::time::Duration::from_millis(150),
        "shutdown_blocking must wait for slow checkpoint (elapsed={elapsed:?})"
    );
    let wal_path = dir.path().join("slow.db-wal");
    if wal_path.exists() {
        assert_eq!(std::fs::metadata(&wal_path).unwrap().len(), 0);
    }
}

#[test]
fn try_write_on_open_writer_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let writer = DbWriter::open(&dir.path().join("t.db"), 64).unwrap();
    let accepted = writer.try_write(WriteOp::FileEvent(crate::events::FileEvent {
        event_id: None,
        timestamp: std::time::SystemTime::now(),
        action: crate::events::FileAction::Created,
        path: "/x".into(),
        size: None,
        trace_id: None,
        credential_ref: None,
    }));
    assert!(accepted);
}

#[test]
fn db_writer_records_enqueue_batch_and_shutdown_metrics() {
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};

    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let (tx, rx) = tokio::sync::mpsc::channel(16);
    tx.blocking_send(super::WriterMessage::Op(WriteOp::FileEvent(
        crate::events::FileEvent {
            event_id: None,
            timestamp: std::time::SystemTime::now(),
            action: crate::events::FileAction::Created,
            path: "/metrics".into(),
            size: None,
            trace_id: None,
            credential_ref: None,
        },
    )))
    .unwrap();
    drop(tx);

    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::schema::apply_pragmas(&conn).unwrap();
    crate::schema::create_tables(&conn).unwrap();
    crate::schema::migrate(&conn);

    metrics::with_local_recorder(&recorder, || writer_loop(conn, rx));

    let snapshot = snapshotter.snapshot().into_vec();
    assert!(snapshot.iter().any(
        |(key, _, _, value)| key.key().name() == DB_WRITE_BATCH_TOTAL
            && matches!(value, DebugValue::Counter(1))
    ));
    assert!(snapshot.iter().any(|(key, _, _, value)| {
        key.key().name() == DB_WRITE_BATCH_DURATION_MS && matches!(value, DebugValue::Histogram(_))
    }));
    assert!(snapshot.iter().any(|(key, _, _, value)| {
        key.key().name() == DB_WRITE_BATCH_SIZE && matches!(value, DebugValue::Histogram(_))
    }));
    assert!(snapshot.iter().any(|(key, _, _, value)| {
        key.key().name() == DB_SHUTDOWN_FLUSH_MS && matches!(value, DebugValue::Histogram(_))
    }));
}

#[test]
fn db_writer_records_enqueue_metrics() {
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};

    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _guard = metrics::set_default_local_recorder(&recorder);

    let dir = tempfile::tempdir().unwrap();
    let writer = DbWriter::open(&dir.path().join("enqueue.db"), 64).unwrap();
    let accepted = writer.try_write(WriteOp::FileEvent(crate::events::FileEvent {
        event_id: None,
        timestamp: std::time::SystemTime::now(),
        action: crate::events::FileAction::Created,
        path: "/enqueue".into(),
        size: None,
        trace_id: None,
        credential_ref: None,
    }));
    assert!(accepted);
    writer.shutdown_blocking();

    let snapshot = snapshotter.snapshot().into_vec();
    assert!(snapshot.iter().any(|(key, _, _, value)| {
        key.key().name() == DB_ENQUEUE_WAIT_MS && matches!(value, DebugValue::Histogram(_))
    }));
}

#[test]
fn write_blocking_persists_without_try_drop() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("blocking.db");
    let writer = DbWriter::open(&db_path, 1).unwrap();
    writer.write_blocking(WriteOp::FileEvent(crate::events::FileEvent {
        event_id: None,
        timestamp: std::time::SystemTime::now(),
        action: crate::events::FileAction::Created,
        path: "/blocking".into(),
        size: None,
        trace_id: None,
        credential_ref: None,
    }));
    writer.shutdown_blocking();

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM fs_events WHERE path = '/blocking'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn write_blocking_is_safe_inside_tokio_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("runtime-blocking.db");
    let writer = DbWriter::open(&db_path, 1).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    rt.block_on(async {
        writer.write_blocking(WriteOp::FileEvent(crate::events::FileEvent {
            event_id: None,
            timestamp: std::time::SystemTime::now(),
            action: crate::events::FileAction::Created,
            path: "/runtime-safe".into(),
            size: None,
            trace_id: None,
            credential_ref: None,
        }));
    });
    writer.shutdown_blocking();

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM fs_events WHERE path = '/runtime-safe'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn brokered_substitution_persists_reference_and_not_secret() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("broker.db");
    let raw_secret = "ghp_raw_secret_that_must_not_be_logged";
    let credential_ref = crate::events::credential_reference("github", raw_secret);

    {
        let writer = DbWriter::open(&db_path, 64).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(async {
            writer
                .write(WriteOp::SubstitutionEvent(
                    crate::events::SubstitutionEvent {
                        event_id: None,
                        timestamp: std::time::SystemTime::now(),
                        material_class: "credential".into(),
                        source: "http.authorization".into(),
                        event_type: Some("http.request".into()),
                        algorithm: "blake3".into(),
                        substitution_ref: credential_ref.clone(),
                        outcome: "captured".into(),
                        provider: Some("github".into()),
                        confidence: Some(1.0),
                        trace_id: Some("trace-credential".into()),
                        context_json: Some(r#"{"header":"authorization"}"#.into()),
                    },
                ))
                .await;
            writer
                .write(WriteOp::NetEvent(crate::events::NetEvent {
                    event_id: None,
                    timestamp: std::time::SystemTime::now(),
                    domain: "api.github.com".into(),
                    port: 443,
                    decision: crate::events::Decision::Allowed,
                    process_name: Some("git".into()),
                    pid: Some(4242),
                    method: Some("GET".into()),
                    path: Some("/repos/openclaw/capsem".into()),
                    query: None,
                    status_code: Some(200),
                    bytes_sent: 128,
                    bytes_received: 512,
                    duration_ms: 30,
                    matched_rule: None,
                    request_headers: Some(format!("authorization: {credential_ref}")),
                    response_headers: None,
                    request_body_preview: None,
                    response_body_preview: None,
                    request_body_full: None,
                    response_body_full: None,
                    conn_type: Some("https".into()),
                    policy_mode: None,
                    policy_action: None,
                    policy_rule: None,
                    policy_reason: None,
                    trace_id: Some("trace-credential".into()),
                    credential_ref: Some(credential_ref.clone()),
                }))
                .await;
        });
    }

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let persisted_ref: String = conn
        .query_row(
            "SELECT credential_ref FROM net_events WHERE domain = 'api.github.com'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(persisted_ref, credential_ref);

    let substitution_ref: String = conn
        .query_row(
            "SELECT substitution_ref FROM substitution_events WHERE source = 'http.authorization'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(substitution_ref, credential_ref);

    for table in ["net_events", "substitution_events"] {
        let sql = format!(
            "SELECT COUNT(*) FROM {table} WHERE CAST({} AS TEXT) LIKE ?1",
            if table == "net_events" {
                "request_headers"
            } else {
                "context_json"
            }
        );
        let leaked: i64 = conn
            .query_row(&sql, [format!("%{raw_secret}%")], |row| row.get(0))
            .unwrap();
        assert_eq!(leaked, 0, "raw secret leaked through {table}");
    }
}

#[test]
fn reader_for_in_memory_writer_fails() {
    let writer = DbWriter::open_in_memory(16).unwrap();
    match writer.reader() {
        Err(rusqlite::Error::InvalidPath(_)) => {}
        Err(other) => panic!("expected InvalidPath, got {other:?}"),
        Ok(_) => panic!("expected reader() to fail for :memory:"),
    }
}

#[test]
fn path_accessor_returns_configured_path() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("mydb.db");
    let writer = DbWriter::open(&p, 16).unwrap();
    assert_eq!(writer.path(), p);
}

#[test]
fn exec_event_insert_then_update_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("exec.db");

    {
        let writer = DbWriter::open(&db_path, 64).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(async {
            writer
                .write(WriteOp::ExecEvent(crate::events::ExecEvent {
                    event_id: None,
                    timestamp: std::time::SystemTime::now(),
                    exec_id: 42,
                    command: "ls -la".into(),
                    source: "mcp".into(),
                    trace_id: Some("t1".into()),
                    process_name: Some("capsem".into()),
                    credential_ref: None,
                }))
                .await;

            writer
                .write(WriteOp::ExecEventComplete(
                    crate::events::ExecEventComplete {
                        exec_id: 42,
                        exit_code: 0,
                        duration_ms: 120,
                        stdout_preview: Some("out".into()),
                        stderr_preview: None,
                        stdout_bytes: 128,
                        stderr_bytes: 0,
                        pid: Some(1234),
                    },
                ))
                .await;
        });
    }

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let (command, source, exit, duration, stdout_preview, stderr_preview, stdout_bytes, pid) = conn.query_row(
        "SELECT command, source, exit_code, duration_ms, stdout_preview, stderr_preview, stdout_bytes, pid
         FROM exec_events WHERE exec_id = 42",
        [],
        |r| {
            let command: String = r.get(0)?;
            let source: String = r.get(1)?;
            let exit: i64 = r.get(2)?;
            let duration: i64 = r.get(3)?;
            let stdout_preview: Option<String> = r.get(4)?;
            let stderr_preview: Option<String> = r.get(5)?;
            let stdout_bytes: i64 = r.get(6)?;
            let pid: Option<i64> = r.get(7)?;
            Ok((command, source, exit, duration, stdout_preview, stderr_preview, stdout_bytes, pid))
        },
    ).unwrap();
    assert_eq!(command, "ls -la");
    assert_eq!(source, "mcp");
    assert_eq!(exit, 0);
    assert_eq!(duration, 120);
    assert_eq!(stdout_preview.as_deref(), Some("out"));
    assert!(stderr_preview.is_none());
    assert_eq!(stdout_bytes, 128);
    assert_eq!(pid, Some(1234));
}

#[test]
fn mcp_call_insert_populates_row() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("mcp.db");

    {
        let writer = DbWriter::open(&db_path, 64).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(async {
            writer
                .write(WriteOp::McpCall(crate::events::McpCall {
                    event_id: None,
                    timestamp: std::time::SystemTime::now(),
                    server_name: "github".into(),
                    method: "tools/call".into(),
                    tool_name: Some("list_issues".into()),
                    request_id: Some("r1".into()),
                    request_preview: Some("{}".into()),
                    response_preview: None,
                    decision: "allowed".into(),
                    duration_ms: 50,
                    error_message: None,
                    process_name: Some("agent".into()),
                    bytes_sent: 64,
                    bytes_received: 128,
                    policy_mode: Some("audit_only".into()),
                    policy_action: Some("allow".into()),
                    policy_rule: Some("mcp.tool.github__list_issues".into()),
                    policy_reason: Some("local policy allow".into()),
                    trace_id: None,
                    credential_ref: None,
                }))
                .await;
        });
    }

    struct ToolCallRow {
        origin: String,
        server: String,
        method: String,
        tool: String,
        decision: String,
        sent: i64,
        recv: i64,
        mode: Option<String>,
        action: Option<String>,
        rule: Option<String>,
        reason: Option<String>,
    }

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let row = conn
        .query_row(
            "SELECT origin, server_name, method, tool_name, decision, bytes_sent, bytes_received,
                policy_mode, policy_action, policy_rule, policy_reason
         FROM tool_calls",
            [],
            |r| {
                Ok(ToolCallRow {
                    origin: r.get(0)?,
                    server: r.get(1)?,
                    method: r.get(2)?,
                    tool: r.get(3)?,
                    decision: r.get(4)?,
                    sent: r.get(5)?,
                    recv: r.get(6)?,
                    mode: r.get(7)?,
                    action: r.get(8)?,
                    rule: r.get(9)?,
                    reason: r.get(10)?,
                })
            },
        )
        .unwrap();
    assert_eq!(row.origin, "mcp");
    assert_eq!(row.server, "github");
    assert_eq!(row.method, "tools/call");
    assert_eq!(row.tool, "list_issues");
    assert_eq!(row.decision, "allowed");
    assert_eq!(row.sent, 64);
    assert_eq!(row.recv, 128);
    assert_eq!(row.mode.as_deref(), Some("audit_only"));
    assert_eq!(row.action.as_deref(), Some("allow"));
    assert_eq!(row.rule.as_deref(), Some("mcp.tool.github__list_issues"));
    assert_eq!(row.reason.as_deref(), Some("local policy allow"));
}

#[test]
fn audit_event_insert_populates_row() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("audit.db");

    {
        let writer = DbWriter::open(&db_path, 64).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(async {
            writer
                .write(WriteOp::AuditEvent(crate::events::AuditEvent {
                    event_id: None,
                    timestamp: std::time::SystemTime::now(),
                    pid: 100,
                    ppid: 1,
                    uid: 501,
                    exe: "/usr/bin/ls".into(),
                    comm: Some("ls".into()),
                    argv: "ls -la".into(),
                    cwd: Some("/tmp".into()),
                    tty: None,
                    session_id: Some(42),
                    audit_id: Some("a1".into()),
                    exec_event_id: Some(7),
                    parent_exe: Some("/bin/bash".into()),
                    trace_id: None,
                    credential_ref: None,
                }))
                .await;
        });
    }

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let (pid, ppid, uid, exe, argv, cwd, parent_exe): (
        i64,
        i64,
        i64,
        String,
        String,
        Option<String>,
        Option<String>,
    ) = conn
        .query_row(
            "SELECT pid, ppid, uid, exe, argv, cwd, parent_exe FROM audit_events",
            [],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(pid, 100);
    assert_eq!(ppid, 1);
    assert_eq!(uid, 501);
    assert_eq!(exe, "/usr/bin/ls");
    assert_eq!(argv, "ls -la");
    assert_eq!(cwd.as_deref(), Some("/tmp"));
    assert_eq!(parent_exe.as_deref(), Some("/bin/bash"));
}

#[test]
fn audit_event_insert_preserves_microsecond_precision() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("audit-precision.db");
    let base = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_713_100_000);

    {
        let writer = DbWriter::open(&db_path, 64).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(async {
            for micros in [123_456_u64, 123_789_u64] {
                writer
                    .write(WriteOp::AuditEvent(crate::events::AuditEvent {
                        event_id: None,
                        timestamp: base + std::time::Duration::from_micros(micros),
                        pid: 100 + micros as u32,
                        ppid: 1,
                        uid: 501,
                        exe: "/usr/bin/ls".into(),
                        comm: Some("ls".into()),
                        argv: "ls -la".into(),
                        cwd: Some("/tmp".into()),
                        tty: None,
                        session_id: Some(42),
                        audit_id: Some(format!("1713100000.{micros}:1")),
                        exec_event_id: None,
                        parent_exe: Some("/bin/bash".into()),
                        trace_id: None,
                        credential_ref: None,
                    }))
                    .await;
            }
        });
    }

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let timestamps = {
        let mut stmt = conn
            .prepare("SELECT timestamp FROM audit_events ORDER BY timestamp ASC")
            .unwrap();
        stmt.query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    assert_eq!(
        timestamps,
        vec!["2024-04-14T13:06:40.123456Z", "2024-04-14T13:06:40.123789Z"]
    );

    let events = crate::DbReader::open(&db_path)
        .unwrap()
        .recent_audit_events(2)
        .unwrap();
    assert_eq!(events.len(), 2);
    assert!(events[0].timestamp > events[1].timestamp);
}

#[test]
fn dns_event_insert_populates_row() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("dns.db");

    {
        let writer = DbWriter::open(&db_path, 64).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(async {
            writer
                .write(WriteOp::DnsEvent(crate::events::DnsEvent {
                    event_id: None,
                    timestamp: std::time::SystemTime::now(),
                    qname: "anthropic.com".into(),
                    qtype: 1,
                    qclass: 1,
                    rcode: 0,
                    answer_ip: Some("93.184.216.34".into()),
                    decision: "allowed".into(),
                    matched_rule: None,
                    source_proto: Some("udp".into()),
                    process_name: None,
                    upstream_resolver_ms: 27,
                    trace_id: Some("abc1234567890def".into()),
                    policy_mode: None,
                    policy_action: None,
                    policy_rule: None,
                    policy_reason: None,
                    credential_ref: None,
                }))
                .await;
            writer
                .write(WriteOp::DnsEvent(crate::events::DnsEvent {
                    event_id: None,
                    timestamp: std::time::SystemTime::now(),
                    qname: "blocked.example.com".into(),
                    qtype: 28,
                    qclass: 1,
                    rcode: 3,
                    answer_ip: None,
                    decision: "denied".into(),
                    matched_rule: Some("*.example.com".into()),
                    source_proto: Some("udp".into()),
                    process_name: None,
                    upstream_resolver_ms: 0,
                    trace_id: None,
                    policy_mode: Some("enforce".into()),
                    policy_action: Some("block".into()),
                    policy_rule: Some("policy.dns.block_example".into()),
                    policy_reason: Some("DNS block from security rule".into()),
                    credential_ref: None,
                }))
                .await;
        });
    }

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let row = |sql: &str| -> (String, i64, i64, i64, String) {
        conn.query_row(sql, [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .unwrap()
    };
    let (qname, qtype, qclass, rcode, decision) = row(
        "SELECT qname, qtype, qclass, rcode, decision FROM dns_events
         WHERE qname = 'anthropic.com'",
    );
    let matched: Option<String> = conn
        .query_row(
            "SELECT matched_rule FROM dns_events WHERE qname = 'anthropic.com'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let (proto, ms, trace): (Option<String>, i64, Option<String>) = conn
        .query_row(
            "SELECT source_proto, upstream_resolver_ms, trace_id FROM dns_events
         WHERE qname = 'anthropic.com'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(qname, "anthropic.com");
    assert_eq!(qtype, 1);
    assert_eq!(qclass, 1);
    assert_eq!(rcode, 0);
    assert_eq!(decision, "allowed");
    assert!(matched.is_none());
    assert_eq!(proto.as_deref(), Some("udp"));
    assert_eq!(ms, 27);
    assert_eq!(trace.as_deref(), Some("abc1234567890def"));

    let (rcode_blocked, matched_blocked): (i64, String) = conn
        .query_row(
            "SELECT rcode, matched_rule FROM dns_events WHERE qname = 'blocked.example.com'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(rcode_blocked, 3);
    assert_eq!(matched_blocked, "*.example.com");

    let (mode, action, rule, reason): (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = conn
        .query_row(
            "SELECT policy_mode, policy_action, policy_rule, policy_reason
             FROM dns_events WHERE qname = 'blocked.example.com'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(mode.as_deref(), Some("enforce"));
    assert_eq!(action.as_deref(), Some("block"));
    assert_eq!(rule.as_deref(), Some("policy.dns.block_example"));
    assert_eq!(reason.as_deref(), Some("DNS block from security rule"));
}

#[test]
fn dns_events_indexed_by_trace_id_for_join() {
    // The promise of T3.3: a single trace_id joins dns_events to
    // net_events for one logical agent action. Verify the index
    // exists so the join is fast even at 100k+ rows.
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("dns_idx.db");
    let _ = DbWriter::open(&db_path, 8).unwrap();
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
         WHERE type='index' AND name='idx_dns_events_trace_id'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "missing idx_dns_events_trace_id");
}
