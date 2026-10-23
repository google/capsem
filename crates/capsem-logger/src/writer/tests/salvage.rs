use super::*;

pub(super) fn file_event_with_credential(path: &str, credential_ref: Option<&str>) -> WriteOp {
    WriteOp::FileEvent(crate::events::FileEvent {
        event_id: None,
        timestamp: std::time::SystemTime::now(),
        action: crate::events::FileAction::Created,
        path: path.to_string(),
        size: Some(1),
        kind: crate::events::FileKind::File,
        trace_id: None,
        credential_ref: credential_ref.map(str::to_string),
    })
}

/// One row the schema rejects must not take its neighbours down with it.
///
/// The write batch is a single transaction for throughput, so a CHECK violation
/// on one op used to roll back every op batched alongside it -- valid security
/// telemetry discarded because an unrelated producer built one bad row, and only
/// a warn line to show for it.
#[test]
fn one_rejected_op_does_not_discard_the_rest_of_its_batch() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = DbWriter::open(&db_path, 64).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();

    rt.block_on(async {
        // Enqueued together so they land in one batch. The middle op carries a
        // credential_ref the schema CHECK rejects.
        writer.write(file_event_with_credential("/good/before", None)).await;
        writer
            .write(file_event_with_credential(
                "/bad/malformed",
                Some("credential:blake3:not-a-real-digest"),
            ))
            .await;
        writer.write(file_event_with_credential("/good/after", None)).await;
        writer.flush().await;
    });
    writer.shutdown_blocking();

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let paths = conn
        .prepare("SELECT path FROM fs_events ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(
        paths,
        vec!["/good/before".to_string(), "/good/after".to_string()],
        "the two valid rows must survive the one the schema refused"
    );
}

pub(super) fn minimal_model_call(trace_id: &str) -> WriteOp {
    WriteOp::ModelCall(crate::events::ModelCall {
        event_id: None,
        timestamp: std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000),
        provider: "anthropic".to_string(),
        protocol: Some("anthropic".to_string()),
        model: Some("claude-sonnet-4-20250514".to_string()),
        process_name: None,
        pid: None,
        method: "POST".to_string(),
        path: "/v1/messages".to_string(),
        stream: false,
        system_prompt_preview: None,
        messages_count: 1,
        tools_count: 0,
        request_bytes: 16,
        request_body: None,
        message_id: Some("msg_salvage".to_string()),
        status_code: Some(200),
        text_content: Some("hello".to_string()),
        thinking_content: None,
        response_body: None,
        stop_reason: Some("end_turn".to_string()),
        input_tokens: Some(1),
        output_tokens: Some(1),
        usage_details: std::collections::BTreeMap::new(),
        duration_ms: 1,
        response_bytes: 16,
        estimated_cost_usd: Some(0.0),
        trace_id: Some(trace_id.to_string()),
        credential_ref: None,
        tool_calls: vec![],
        tool_responses: vec![],
    })
}

#[test]
fn salvaging_a_failed_batch_preserves_model_items() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = DbWriter::open(&db_path, 64).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();

    rt.block_on(async {
        writer.write(minimal_model_call("trace_salvage")).await;
        writer
            .write(file_event_with_credential(
                "/bad/malformed",
                Some("credential:blake3:not-a-real-digest"),
            ))
            .await;
        writer.flush().await;
    });
    writer.shutdown_blocking();

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let calls: i64 = conn
        .query_row("SELECT count(*) FROM model_calls", [], |row| row.get(0))
        .unwrap();
    let items: i64 = conn
        .query_row("SELECT count(*) FROM model_items", [], |row| row.get(0))
        .unwrap();

    assert_eq!(calls, 1, "the model call survives the rejected row beside it");
    assert!(
        items > 0,
        "the model items must survive retrying the rejected batch one row at a time"
    );
}

#[test]
fn a_rejected_op_is_dropped_wherever_it_sits_in_the_batch() {
    for (label, position) in [("first", 0usize), ("middle", 1), ("last", 2)] {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("session.db");
        let writer = DbWriter::open(&db_path, 64).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();

        rt.block_on(async {
            for index in 0..3 {
                if index == position {
                    writer
                        .write(file_event_with_credential(
                            "/bad/malformed",
                            Some("credential:blake3:not-a-real-digest"),
                        ))
                        .await;
                } else {
                    writer
                        .write(file_event_with_credential(&format!("/good/{index}"), None))
                        .await;
                }
            }
            writer.flush().await;
        });
        writer.shutdown_blocking();

        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let surviving: i64 = conn
            .query_row("SELECT count(*) FROM fs_events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            surviving, 2,
            "a rejected op in the {label} position must cost only itself"
        );
    }
}

#[test]
fn a_batch_of_only_rejected_ops_writes_nothing_and_does_not_wedge_the_writer() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = DbWriter::open(&db_path, 64).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();

    rt.block_on(async {
        for _ in 0..3 {
            writer
                .write(file_event_with_credential(
                    "/bad/malformed",
                    Some("credential:blake3:not-a-real-digest"),
                ))
                .await;
        }
        writer.flush().await;
        // The writer must still accept work after salvaging an all-bad batch.
        writer.write(file_event_with_credential("/good/after", None)).await;
        writer.flush().await;
    });
    writer.shutdown_blocking();

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let paths = conn
        .prepare("SELECT path FROM fs_events ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(paths, vec!["/good/after".to_string()]);
}
