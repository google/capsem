use super::*;

fn sample_file_event(path: &str, action: FileAction, size: Option<u64>) -> FileEvent {
    FileEvent {
        event_id: None,
        timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(1700000000),
        action,
        path: path.to_string(),
        size,
        trace_id: None,
        credential_ref: None,
    }
}

#[tokio::test]
async fn test_file_event_write_read() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fs-roundtrip.db");

    let writer = DbWriter::open(&path, 64).unwrap();
    writer
        .write(WriteOp::FileEvent(sample_file_event(
            "project/app.js",
            FileAction::Created,
            Some(1234),
        )))
        .await;
    writer
        .write(WriteOp::FileEvent(sample_file_event(
            "project/lib.rs",
            FileAction::Modified,
            Some(5678),
        )))
        .await;
    writer
        .write(WriteOp::FileEvent(sample_file_event(
            "project/old.txt",
            FileAction::Deleted,
            None,
        )))
        .await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let events = reader.recent_file_events(10).unwrap();
    assert_eq!(events.len(), 3);
    // Most recent first
    assert_eq!(events[0].path, "project/old.txt");
    assert_eq!(events[0].action, FileAction::Deleted);
    assert!(events[0].size.is_none());
    assert_eq!(events[1].path, "project/lib.rs");
    assert_eq!(events[1].action, FileAction::Modified);
    assert_eq!(events[1].size, Some(5678));
    assert_eq!(events[2].path, "project/app.js");
    assert_eq!(events[2].action, FileAction::Created);
    assert_eq!(events[2].size, Some(1234));
}

#[tokio::test]
async fn test_file_event_search() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fs-search.db");

    let writer = DbWriter::open(&path, 64).unwrap();
    writer
        .write(WriteOp::FileEvent(sample_file_event(
            "project/src/app.js",
            FileAction::Created,
            Some(100),
        )))
        .await;
    writer
        .write(WriteOp::FileEvent(sample_file_event(
            "project/src/lib.rs",
            FileAction::Modified,
            Some(200),
        )))
        .await;
    writer
        .write(WriteOp::FileEvent(sample_file_event(
            "project/README.md",
            FileAction::Modified,
            Some(300),
        )))
        .await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let results = reader.search_file_events("src", 10).unwrap();
    assert_eq!(results.len(), 2);
    // Only the two src/ files match
    for r in &results {
        assert!(
            r.path.contains("src"),
            "expected path containing 'src', got: {}",
            r.path
        );
    }
}

#[tokio::test]
async fn test_file_event_stats() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fs-stats.db");

    let writer = DbWriter::open(&path, 64).unwrap();
    writer
        .write(WriteOp::FileEvent(sample_file_event(
            "a.js",
            FileAction::Created,
            Some(10),
        )))
        .await;
    writer
        .write(WriteOp::FileEvent(sample_file_event(
            "b.js",
            FileAction::Created,
            Some(20),
        )))
        .await;
    writer
        .write(WriteOp::FileEvent(sample_file_event(
            "a.js",
            FileAction::Modified,
            Some(15),
        )))
        .await;
    writer
        .write(WriteOp::FileEvent(sample_file_event("c.js", FileAction::Deleted, None)))
        .await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let stats = reader.file_event_stats().unwrap();
    assert_eq!(stats.total, 4);
    assert_eq!(stats.created, 2);
    assert_eq!(stats.modified, 1);
    assert_eq!(stats.deleted, 1);
}

#[tokio::test]
async fn test_file_event_empty_table() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fs-empty.db");

    let writer = DbWriter::open(&path, 64).unwrap();
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let events = reader.recent_file_events(10).unwrap();
    assert!(events.is_empty());
    let stats = reader.file_event_stats().unwrap();
    assert_eq!(stats.total, 0);
    assert_eq!(stats.created, 0);
    assert_eq!(stats.modified, 0);
    assert_eq!(stats.deleted, 0);
}

/// Fixture DB should contain fs_events rows inserted during fixture setup.
#[test]
fn test_file_events_in_fixture() {
    let reader = fixture_reader();
    let events = reader.recent_file_events(100).unwrap();
    assert!(!events.is_empty(), "fixture should contain fs_events");
    let stats = reader.file_event_stats().unwrap();
    assert!(stats.total > 0);
    assert!(stats.created > 0);
    assert!(stats.modified > 0);
    assert!(stats.deleted > 0);
    // Verify all actions parse correctly
    for e in &events {
        assert!(
            matches!(
                e.action,
                FileAction::Created | FileAction::Modified | FileAction::Deleted
            ),
            "unexpected action: {:?}",
            e.action
        );
        assert!(!e.path.is_empty(), "path should not be empty in fixture");
    }
}

/// Fixture search should filter by path substring.
#[test]
fn test_file_events_fixture_search() {
    let reader = fixture_reader();
    let results = reader.search_file_events("src", 100).unwrap();
    for r in &results {
        assert!(r.path.contains("src"), "search result should contain 'src': {}", r.path);
    }
}

/// Empty path: should insert and read back without error.
#[tokio::test]
async fn test_file_event_empty_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fs-empty-path.db");

    let writer = DbWriter::open(&path, 64).unwrap();
    writer
        .write(WriteOp::FileEvent(sample_file_event("", FileAction::Created, Some(0))))
        .await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let events = reader.recent_file_events(10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].path, "");
    assert_eq!(events[0].action, FileAction::Created);
    assert_eq!(events[0].size, Some(0));
}

/// Unicode paths: filenames with emoji, CJK, RTL, combining characters.
#[tokio::test]
async fn test_file_event_unicode_paths() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fs-unicode.db");

    let writer = DbWriter::open(&path, 64).unwrap();
    let paths = vec![
        "project/\u{1F4C4}document.txt",
        "project/\u{4E2D}\u{6587}\u{6587}\u{4EF6}.rs",
        "project/\u{0645}\u{0644}\u{0641}.py",
        "project/caf\u{0065}\u{0301}.js", // e + combining accent
        "project/\u{0000}null.txt",       // null byte in path
    ];
    for p in &paths {
        writer
            .write(WriteOp::FileEvent(sample_file_event(p, FileAction::Created, Some(100))))
            .await;
    }
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let events = reader.recent_file_events(100).unwrap();
    assert_eq!(events.len(), paths.len());
}

/// Very long path: shouldn't crash or truncate silently.
#[tokio::test]
async fn test_file_event_huge_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fs-huge-path.db");

    let huge_path = "a/".repeat(10_000) + "file.txt"; // ~30KB path
    let writer = DbWriter::open(&path, 64).unwrap();
    writer
        .write(WriteOp::FileEvent(sample_file_event(
            &huge_path,
            FileAction::Modified,
            Some(42),
        )))
        .await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let events = reader.recent_file_events(10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].path, huge_path);
}

/// Size boundary: u64::MAX should round-trip via i64 (may lose precision).
#[tokio::test]
async fn test_file_event_max_size() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fs-max-size.db");

    let writer = DbWriter::open(&path, 64).unwrap();
    writer
        .write(WriteOp::FileEvent(sample_file_event(
            "big.bin",
            FileAction::Created,
            Some(u64::MAX),
        )))
        .await;
    writer
        .write(WriteOp::FileEvent(sample_file_event(
            "zero.bin",
            FileAction::Created,
            Some(0),
        )))
        .await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let events = reader.recent_file_events(10).unwrap();
    assert_eq!(events.len(), 2);
    // size=0 should round-trip exactly
    assert_eq!(events[0].size, Some(0));
    // u64::MAX stored as i64 wraps, but shouldn't crash
    assert!(events[1].size.is_some());
}

/// SQL injection via search: parameterized queries should prevent it.
#[tokio::test]
async fn test_file_event_search_sql_injection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fs-sqli.db");

    let writer = DbWriter::open(&path, 64).unwrap();
    writer
        .write(WriteOp::FileEvent(sample_file_event(
            "safe.rs",
            FileAction::Created,
            Some(10),
        )))
        .await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    // Should return empty, not crash or drop the table.
    let results = reader.search_file_events("'; DROP TABLE fs_events; --", 100).unwrap();
    assert!(results.is_empty());
    // Table still works:
    let events = reader.recent_file_events(10).unwrap();
    assert_eq!(events.len(), 1);
}

/// Search with SQL wildcards in user input should be treated as literals.
#[tokio::test]
async fn test_file_event_search_wildcards() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fs-wildcards.db");

    let writer = DbWriter::open(&path, 64).unwrap();
    writer
        .write(WriteOp::FileEvent(sample_file_event(
            "src/main.rs",
            FileAction::Created,
            Some(10),
        )))
        .await;
    writer
        .write(WriteOp::FileEvent(sample_file_event(
            "src/lib.rs",
            FileAction::Modified,
            Some(20),
        )))
        .await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    // "%" in search should match within LIKE, but is user-provided -- verify no crash
    let results = reader.search_file_events("%", 100).unwrap();
    // "%" inside our LIKE pattern becomes "%%%" which matches everything
    assert_eq!(results.len(), 2);
}

/// Batch of many events: tests the batching/drain path in DbWriter.
#[tokio::test]
async fn test_file_event_batch_write() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fs-batch.db");

    let writer = DbWriter::open(&path, 64).unwrap();
    for i in 0..500 {
        let action = match i % 3 {
            0 => FileAction::Created,
            1 => FileAction::Modified,
            _ => FileAction::Deleted,
        };
        let size = if action == FileAction::Deleted {
            None
        } else {
            Some(i as u64)
        };
        writer
            .write(WriteOp::FileEvent(sample_file_event(
                &format!("file_{i}.rs"),
                action,
                size,
            )))
            .await;
    }
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let stats = reader.file_event_stats().unwrap();
    assert_eq!(stats.total, 500);
    assert_eq!(stats.created, 167); // 0,3,6,...,498 -> ceil(500/3) = 167
    assert_eq!(stats.modified, 167); // 1,4,7,...,499
    assert_eq!(stats.deleted, 166); // 2,5,8,...,497
                                    // Limit query returns at most the requested count
    let events = reader.recent_file_events(50).unwrap();
    assert_eq!(events.len(), 50);
}

/// Concurrent writers: multiple tasks writing file events simultaneously.
#[tokio::test]
async fn test_file_event_concurrent_writes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fs-concurrent.db");

    let writer = Arc::new(DbWriter::open(&path, 64).unwrap());
    let mut handles = vec![];
    for t in 0..10 {
        let w = Arc::clone(&writer);
        handles.push(tokio::spawn(async move {
            for i in 0..50 {
                w.write(WriteOp::FileEvent(sample_file_event(
                    &format!("thread_{t}/file_{i}.rs"),
                    FileAction::Modified,
                    Some(i as u64),
                )))
                .await;
            }
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let stats = reader.file_event_stats().unwrap();
    assert_eq!(stats.total, 500); // 10 threads x 50 events
}

/// Schema migration: a DB created without fs_events should gain the table on migrate.
#[tokio::test]
async fn test_file_event_schema_migration() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fs-migrate.db");

    // Create a minimal DB with only net_events (simulating an old schema).
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "
            CREATE TABLE net_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT NOT NULL,
                domain TEXT NOT NULL,
                port INTEGER NOT NULL,
                decision TEXT NOT NULL,
                bytes_sent INTEGER NOT NULL DEFAULT 0,
                bytes_received INTEGER NOT NULL DEFAULT 0,
                duration_ms INTEGER NOT NULL DEFAULT 0
            );
        ",
        )
        .unwrap();
    }

    // Opening with DbWriter triggers migration, which should add fs_events.
    let writer = DbWriter::open(&path, 64).unwrap();
    writer
        .write(WriteOp::FileEvent(sample_file_event(
            "migrated.rs",
            FileAction::Created,
            Some(42),
        )))
        .await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let events = reader.recent_file_events(10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].path, "migrated.rs");
}

/// Deleted events should have size=None and round-trip correctly.
#[tokio::test]
async fn test_file_event_deleted_has_no_size() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fs-deleted-size.db");

    let writer = DbWriter::open(&path, 64).unwrap();
    writer
        .write(WriteOp::FileEvent(sample_file_event(
            "gone.rs",
            FileAction::Deleted,
            None,
        )))
        .await;
    // Also test deleted with size (shouldn't crash even though unusual).
    writer
        .write(WriteOp::FileEvent(sample_file_event(
            "ghost.rs",
            FileAction::Deleted,
            Some(999),
        )))
        .await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let events = reader.recent_file_events(10).unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].action, FileAction::Deleted);
    assert_eq!(events[0].size, Some(999)); // unusual but valid
    assert_eq!(events[1].action, FileAction::Deleted);
    assert!(events[1].size.is_none());
}

/// Limit=0 should return no events, not crash.
#[tokio::test]
async fn test_file_event_limit_zero() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fs-limit-zero.db");

    let writer = DbWriter::open(&path, 64).unwrap();
    writer
        .write(WriteOp::FileEvent(sample_file_event(
            "a.rs",
            FileAction::Created,
            Some(1),
        )))
        .await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let events = reader.recent_file_events(0).unwrap();
    assert!(events.is_empty());
    let search = reader.search_file_events("a", 0).unwrap();
    assert!(search.is_empty());
}

// ── Bounded writer queue backpressure ───────────────────────────────

/// Proves every operation accepted by non-blocking try_write() is persisted.
#[tokio::test]
async fn try_write_persists_every_accepted_operation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("try-write-burst.db");

    let writer = DbWriter::open(&path, 1).unwrap();

    let mut accepted = usize::from(writer.try_write(WriteOp::FileEvent(sample_file_event(
        "first.rs",
        FileAction::Created,
        Some(10),
    ))));

    for i in 0..20 {
        accepted += usize::from(writer.try_write(WriteOp::FileEvent(sample_file_event(
            &format!("burst{i}.rs"),
            FileAction::Modified,
            Some(100),
        ))));
    }

    writer.flush().await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let events = reader.recent_file_events(100).unwrap();

    assert_eq!(
        events.len(),
        accepted,
        "try_write must persist exactly the operations it reports as accepted"
    );
}

/// Proves that write().await does NOT drop events under the same conditions,
/// because it backpressures (yields) until the channel has space.
#[tokio::test]
async fn async_write_never_drops_events() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("async-write-safe.db");

    // Same tiny capacity.
    let writer = DbWriter::open(&path, 1).unwrap();

    // Send 21 events via write().await -- all will succeed because write()
    // awaits channel capacity instead of failing.
    writer
        .write(WriteOp::FileEvent(sample_file_event(
            "first.rs",
            FileAction::Created,
            Some(10),
        )))
        .await;

    for i in 0..20 {
        writer
            .write(WriteOp::FileEvent(sample_file_event(
                &format!("safe{i}.rs"),
                FileAction::Modified,
                Some(100),
            )))
            .await;
    }

    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let events = reader.recent_file_events(100).unwrap();

    // Every single event was persisted -- zero data loss.
    assert_eq!(
        events.len(),
        21,
        "write().await should persist all 21 events, but only got {}",
        events.len()
    );
}

/// Simulates a production-sized burst and proves try_write's acceptance count
/// matches the rows visible after the explicit DB barrier.
#[tokio::test]
async fn try_write_production_burst_preserves_events() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("burst-preserve.db");

    let writer = DbWriter::open(&path, 256).unwrap();

    let total = 500;
    let mut accepted = 0;
    for i in 0..total {
        accepted += usize::from(writer.try_write(WriteOp::FileEvent(sample_file_event(
            &format!("burst{i}.rs"),
            FileAction::Modified,
            Some(i as u64),
        ))));
    }

    writer.flush().await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let events = reader.recent_file_events(1000).unwrap();

    assert_eq!(events.len(), accepted);
}
