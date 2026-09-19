use super::*;

// -----------------------------------------------------------------------
// Body retention on stop
//
// A persistent VM's session directory outlives every stop, so its body
// archive is the one thing in the ledger that grows without bound. This
// process owns the writer, so this is the only place it can be trimmed.
// -----------------------------------------------------------------------

/// Write one archived body and flush it into the open block.
async fn archive_one_body(db: &DbWriter, event_id: &str, payload: &str) {
    db.write_checked(capsem_logger::WriteOp::SecurityRuleEvent(
        capsem_logger::SecurityRuleEvent::new(
            1_789_000_223_456,
            event_id,
            "model.call",
            "profiles.rules.example",
            r#"{"name":"example"}"#,
            payload,
        ),
    ))
    .await
    .expect("write a security rule event");
    db.flush_checked().await.expect("flush");
}

fn archived_blocks(db_path: &std::path::Path) -> usize {
    let reader = capsem_logger::DbReader::open(db_path).expect("open the ledger");
    let raw = reader
        .query_raw_with_params("SELECT COUNT(*) FROM body_blocks", &[])
        .expect("count archived blocks");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("count json");
    value["rows"][0][0].as_u64().expect("a count") as usize
}

#[tokio::test]
async fn stopping_a_persistent_session_drops_bodies_past_the_retention_period() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let db = DbWriter::open(&db_path, 16).unwrap();
    archive_one_body(&db, "abcdef123450", r#"{"from":"an old session"}"#).await;
    archive_one_body(&db, "abcdef123451", r#"{"from":"this session"}"#).await;
    assert_eq!(archived_blocks(&db_path), 1, "both flushes grew the one open block");

    // Zero days of retention is every block last written before this instant,
    // which is all of them: the period is what selects, and this proves it selects.
    retain_session_bodies(&db, 0).await;

    assert_eq!(
        archived_blocks(&db_path),
        0,
        "blocks past the retention period are dropped with their index rows"
    );
}

#[tokio::test]
async fn retention_keeps_the_bodies_inside_the_period() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let db = DbWriter::open(&db_path, 16).unwrap();
    archive_one_body(&db, "abcdef123450", r#"{"from":"this session"}"#).await;

    retain_session_bodies(&db, 30).await;

    assert_eq!(
        archived_blocks(&db_path),
        1,
        "a body written moments ago is not 30 days old"
    );
    let reader = capsem_logger::DbReader::open(&db_path).expect("open the ledger");
    let raw = reader
        .query_raw_with_params("SELECT COUNT(*) FROM event_body_blobs", &[])
        .expect("count index rows");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("count json");
    assert_eq!(value["rows"][0][0].as_u64(), Some(1), "and its index row stays with it");
}
