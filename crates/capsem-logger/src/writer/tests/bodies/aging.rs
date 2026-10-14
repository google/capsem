use super::*;

/// Seconds since the epoch the test clock reads, so a test can move time.
static TEST_CLOCK_SECS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1_800_000_000);

fn test_clock() -> SystemTime {
    SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(TEST_CLOCK_SECS.load(std::sync::atomic::Ordering::SeqCst))
}

fn advance_test_clock(by: std::time::Duration) {
    TEST_CLOCK_SECS.fetch_add(by.as_secs(), std::sync::atomic::Ordering::SeqCst);
}

/// A quiet session must not keep one block open for days: retention drops
/// whole blocks by their last write, so a block that never closed would keep
/// every body in it for as long as anything new joined it. A block older
/// than `MAX_BLOCK_AGE` closes, and the next body starts a new one.
#[test]
fn a_block_open_longer_than_its_age_limit_closes_before_the_next_body() {
    use crate::writer::bodies::MAX_BLOCK_AGE;

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("block-age.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    crate::schema::create_tables(&conn).unwrap();

    let mut archive = BodyArchive::open_for_tests(Some(&db_path), test_clock, &conn);
    archive.stage(&conn, body_blob("0a0a0a0a0b01", "opens the block"));
    archive.flush_segment();
    advance_test_clock(MAX_BLOCK_AGE.saturating_sub(std::time::Duration::from_secs(60)));
    archive.stage(&conn, body_blob("0a0a0a0a0b02", "still inside the age limit"));
    archive.flush_segment();
    advance_test_clock(std::time::Duration::from_secs(120));
    archive.stage(&conn, body_blob("0a0a0a0a0b03", "past it"));
    archive.flush_segment();
    commit(&mut archive, &conn);

    let blocks: Vec<i64> = conn
        .prepare("SELECT block_offset FROM event_body_blobs ORDER BY event_id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(blocks[0], blocks[1], "inside the limit, one block");
    assert!(blocks[2] > blocks[1], "past it, a new block: {blocks:?}");
    assert_eq!(read_indexed(&conn, &db_path, "0a0a0a0a0b03"), b"past it");

    // And an idle block closes at the next flush, not only at the next body.
    advance_test_clock(MAX_BLOCK_AGE);
    archive.flush_segment();
    assert_eq!(archive.appended_len_for_tests(), 1, "the stale block closed");
    commit(&mut archive, &conn);
}
