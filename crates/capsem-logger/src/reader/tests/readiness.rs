//! Readiness is a structural question and must cost what a structural question
//! costs: a read of the schema, not a walk of the ledger.
//!
//! It used to open with `PRAGMA integrity_check`, which visits every page and
//! every index entry. A failed readiness is retried by its caller, so a broken
//! ledger was fully scanned on every poll, and the first readiness of a long
//! session paid for the whole file before a route could answer.

use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// A ledger created the way production creates one -- schema and archive
/// together -- then grown with `rows` DNS rows straight into the file.
fn ledger_with_rows(dir: &std::path::Path, rows: usize) -> std::path::PathBuf {
    let path = dir.join(format!("ledger-{rows}.db"));
    seed_ledger(&path, rows);
    path
}

fn seed_ledger(path: &std::path::Path, rows: usize) {
    drop(crate::DbWriter::open(path, 1).expect("create ledger"));
    let mut conn = Connection::open(path).expect("open seed connection");
    let tx = conn.transaction().expect("seed transaction");
    {
        let mut insert = tx
            .prepare(
                "INSERT INTO dns_events (
                    event_id, timestamp, qname, qtype, qclass, rcode, decision, upstream_resolver_ms,
                    trace_id, turn_id
                 ) VALUES (?1, '1970-01-01T00:00:00Z', ?2, 1, 1, 0, 'allowed', 0, ?1, ?1)",
            )
            .expect("prepare seed insert");
        for idx in 0..rows {
            insert
                .execute(rusqlite::params![format!("{idx:012x}"), format!("r{idx}.example")])
                .expect("seed row");
        }
    }
    tx.commit().expect("commit seed rows");
}

/// SQLite virtual-machine steps one readiness check executes.
///
/// Steps, not time: a full scan is proportional to the rows it visits and the
/// count says so on any machine, where a timing bound would only say it on a
/// quiet one.
fn readiness_steps(reader: &DbReader) -> u64 {
    let steps = Arc::new(AtomicU64::new(0));
    let counter = Arc::clone(&steps);
    reader.connection().progress_handler(
        1,
        Some(move || {
            counter.fetch_add(1, Ordering::Relaxed);
            false
        }),
    );
    reader.ready().expect("a healthy ledger is ready");
    reader.connection().progress_handler(0, None::<fn() -> bool>);
    steps.load(Ordering::Relaxed)
}

#[test]
fn readiness_cost_does_not_grow_with_the_ledger() {
    let dir = tempfile::tempdir().unwrap();
    let small = DbReader::open(&ledger_with_rows(dir.path(), 10)).unwrap();
    let large = DbReader::open(&ledger_with_rows(dir.path(), 20_000)).unwrap();

    let small_steps = readiness_steps(&small);
    let large_steps = readiness_steps(&large);

    // Two thousand times the rows; the schema is identical, so the check must
    // be too. Any per-row work shows up here as orders of magnitude.
    assert_eq!(
        large_steps, small_steps,
        "readiness walked the ledger: {small_steps} steps at 10 rows, {large_steps} at 20000"
    );
}

#[test]
fn repeated_readiness_validates_the_shape_once_per_schema_version() {
    let dir = tempfile::tempdir().unwrap();
    let path = ledger_with_rows(dir.path(), 10);
    let reader = DbReader::open(&path).unwrap();

    for _ in 0..3 {
        reader.ready().expect("healthy ledger is ready");
    }
    assert_eq!(reader.shape_validations(), 1, "an unchanged schema is validated once");

    // Row commits are not schema changes and cost the check nothing.
    ledger_rows_append(&path, 5);
    reader.ready().expect("still ready after a data commit");
    assert_eq!(reader.shape_validations(), 1, "a data commit does not re-validate");

    // DDL moves `schema_version`, and a changed schema is validated again.
    Connection::open(&path)
        .unwrap()
        .execute_batch("CREATE TABLE unrelated_extension (id INTEGER PRIMARY KEY);")
        .unwrap();
    reader.ready().expect("an added table leaves the ledger ready");
    assert_eq!(reader.shape_validations(), 2, "DDL re-validates the shape");
}

#[test]
fn readiness_notices_a_required_table_dropped_after_it_passed() {
    let dir = tempfile::tempdir().unwrap();
    let path = ledger_with_rows(dir.path(), 10);
    let reader = DbReader::open(&path).unwrap();
    reader.ready().expect("healthy ledger is ready");

    Connection::open(&path)
        .unwrap()
        .execute_batch("DROP TABLE profile_mutation_events;")
        .unwrap();

    // A verdict cached forever would still answer "ready" here.
    for _ in 0..2 {
        let error = reader.ready().expect_err("a dropped required table is not ready");
        assert!(
            error.contains("profile_mutation_events"),
            "the failure names the missing table: {error}"
        );
    }
}

#[test]
fn failed_readiness_stays_a_structural_check() {
    let dir = tempfile::tempdir().unwrap();
    let path = ledger_with_rows(dir.path(), 20_000);
    Connection::open(&path)
        .unwrap()
        .execute_batch("DROP TABLE profile_mutation_events;")
        .unwrap();
    let broken = DbReader::open(&path).unwrap();
    let healthy = DbReader::open(&ledger_with_rows(dir.path(), 10)).unwrap();
    let healthy_steps = readiness_steps(&healthy);

    // A failure is retried by every caller that polls, so it must not be the
    // expensive path. It stops at the first missing table, which is no more
    // work than a full healthy check.
    let steps = Arc::new(AtomicU64::new(0));
    let counter = Arc::clone(&steps);
    broken.connection().progress_handler(
        1,
        Some(move || {
            counter.fetch_add(1, Ordering::Relaxed);
            false
        }),
    );
    for _ in 0..3 {
        broken.ready().expect_err("broken ledger is not ready");
    }
    let per_failure = steps.load(Ordering::Relaxed) / 3;
    assert!(
        per_failure <= healthy_steps,
        "a failed readiness did more work ({per_failure} steps) than a healthy one ({healthy_steps})"
    );
}

fn ledger_rows_append(path: &std::path::Path, rows: usize) {
    let conn = Connection::open(path).unwrap();
    for idx in 0..rows {
        conn.execute(
            "INSERT INTO dns_events (
                event_id, timestamp, qname, qtype, qclass, rcode, decision, upstream_resolver_ms
             ) VALUES (?1, '1970-01-01T00:00:00Z', 'appended.example', 1, 1, 0, 'allowed', 0)",
            [format!("a{idx:011x}")],
        )
        .unwrap();
    }
}

/// Readiness no longer scans, so a damaged page must still fail loudly
/// somewhere: at the query that reads it, and at the ledger copy, which
/// checks the whole file. Neither may answer as if the rows were not there.
#[tokio::test]
async fn a_corrupt_page_fails_its_reads_and_the_ledger_copy_loudly() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir(&src).unwrap();
    let path = src.join("session.db");
    seed_ledger(&path, 5_000);

    let (root_page, page_size) = {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").unwrap();
        let root: i64 = conn
            .query_row(
                "SELECT rootpage FROM sqlite_master WHERE name = 'dns_events'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let size: i64 = conn.query_row("PRAGMA page_size", [], |row| row.get(0)).unwrap();
        (root as u64, size as u64)
    };
    {
        use std::io::{Seek, SeekFrom, Write};
        let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        // The b-tree page type byte: 0xFF is no page type SQLite knows.
        file.seek(SeekFrom::Start((root_page - 1) * page_size)).unwrap();
        file.write_all(&[0xFF]).unwrap();
    }

    let handle = crate::DbHandle::open_external_reader(&path).unwrap();
    handle
        .ready()
        .await
        .expect("the schema is intact, so the ledger is structurally ready");
    let error = handle
        .query("SELECT SUM(upstream_resolver_ms) AS total FROM dns_events", &[])
        .await
        .expect_err("a read of the damaged table must fail, not come back empty");
    assert!(error.contains("malformed"), "the failure names the damage: {error}");

    let copy = crate::snapshot_session_ledger(&src, &dir.path().join("dst"))
        .expect_err("a damaged ledger is not copied as a trustworthy one");
    assert!(
        format!("{copy:#}").contains("quick_check") || format!("{copy:#}").contains("malformed"),
        "the copy refuses on the integrity check: {copy:#}"
    );
}
