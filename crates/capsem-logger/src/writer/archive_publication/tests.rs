use super::*;

#[test]
fn rewrite_metadata_and_archive_root_use_one_publication_transaction() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::schema::create_tables(&conn).unwrap();
    conn.execute_batch(
        "CREATE TABLE publication_probe(value INTEGER NOT NULL); INSERT INTO publication_probe VALUES(0)",
    )
    .unwrap();
    let old = crate::schema::archive_state(&conn).unwrap();
    let next = capsem_archive::GenerationId::generate();
    let dir = tempfile::tempdir().unwrap();
    publish_transaction(&conn, dir.path(), |tx| {
        tx.execute("UPDATE publication_probe SET value=1", [])?;
        publish_root(tx, old, next, capsem_archive::FILE_HEADER_BYTES as u64, dir.path())
    })
    .unwrap();
    assert_eq!(crate::schema::archive_state(&conn).unwrap().header.generation_id, next);
    assert_eq!(
        conn.query_row("SELECT value FROM publication_probe", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn changed_captured_root_refuses_and_rolls_back_rewrite_metadata() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::schema::create_tables(&conn).unwrap();
    conn.execute_batch(
        "CREATE TABLE publication_probe(value INTEGER NOT NULL); INSERT INTO publication_probe VALUES(0)",
    )
    .unwrap();
    let old = crate::schema::archive_state(&conn).unwrap();
    conn.execute("UPDATE archive_state SET revision=revision+1", [])
        .unwrap();
    let changed = crate::schema::archive_state(&conn).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let result = publish_transaction(&conn, dir.path(), |tx| {
        tx.execute("UPDATE publication_probe SET value=1", [])?;
        publish_root(
            tx,
            old,
            capsem_archive::GenerationId::generate(),
            capsem_archive::FILE_HEADER_BYTES as u64,
            dir.path(),
        )
    });
    assert!(matches!(result, Err(PublishError::BeforeCommit(_))));
    assert_eq!(crate::schema::archive_state(&conn).unwrap(), changed);
    assert_eq!(
        conn.query_row("SELECT value FROM publication_probe", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn failed_rewrite_discards_unflushed_candidate_and_durably_removes_its_name() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let directory = ContainedDir::open_root(dir.path()).unwrap();
    let generation = GenerationId::generate();
    let mut candidate =
        BodyLogWriter::create_generation(&directory, capsem_archive::ArchiveId::generate(), generation).unwrap();
    candidate.stage(b"unpublished candidate bytes").unwrap();
    assert!(candidate.pending_bytes() > 0);
    let outcome = cleanup_unpublished::<()>(
        &directory,
        candidate,
        generation,
        "rewrite",
        "injected logical rewrite failure".into(),
    );
    assert!(outcome
        .into_result()
        .unwrap_err()
        .contains("orphan cleanup pending: false"));
    assert!(!dir.path().join(generation.file_name()).exists());
}
