//! The parts of the export that are a function of a row, not of a ledger.
//!
//! The end-to-end proof is in `db/handle_tests/warc_export.rs`, where there is
//! a session to export. These are the two places a wrong answer would be
//! silent: a date that is off by an hour, and a query whose shape no longer
//! covers every source table.

use super::*;

#[test]
fn a_ledger_timestamp_becomes_whole_seconds_in_utc() {
    for (stamp, expected) in [
        ("2026-09-17T10:11:12.345678Z", "2026-09-17T10:11:12Z"),
        ("2026-09-17T10:11:12Z", "2026-09-17T10:11:12Z"),
        ("2026-09-17T10:11:12.5Z", "2026-09-17T10:11:12Z"),
        ("1999-12-31T23:59:59.999999Z", "1999-12-31T23:59:59Z"),
    ] {
        assert_eq!(warc_date_from_ledger(stamp).as_deref(), Some(expected), "{stamp}");
    }
}

/// A timestamp this code did not actually parse must not be dated anyway.
/// Shifting by an offset nobody read is how an export ends up an hour wrong
/// with nothing in it to say so.
#[test]
fn a_timestamp_that_is_not_a_ledger_timestamp_has_no_date() {
    for stamp in [
        "",
        "2026-09-17",
        "2026-09-17T10:11:12",
        "2026-09-17T10:11:12+02:00",
        "2026-09-17T10:11:12.345678+02:00",
        "2026-09-17 10:11:12Z",
        "20x6-09-17T10:11:12Z",
        "2026-09-17T10:11:12.345678Zjunk",
        "not a time at all",
    ] {
        assert_eq!(warc_date_from_ledger(stamp), None, "{stamp:?} must not produce a date");
    }
}

#[test]
fn unix_milliseconds_become_the_same_whole_seconds() {
    assert_eq!(
        warc_date_from_unix_ms(1_774_346_072_345).as_deref(),
        Some("2026-03-24T09:54:32Z")
    );
    assert_eq!(warc_date_from_unix_ms(0).as_deref(), Some("1970-01-01T00:00:00Z"));
    assert_eq!(warc_date_from_unix_ms(-1), None, "a time before the epoch is not one");
}

/// The metadata lookup has to cover every source table
/// `event_body_blobs` permits. A branch that went missing would not fail
/// anything -- the rows would simply stop appearing in exports.
#[test]
fn the_query_covers_every_source_table_the_schema_allows() {
    let allowed: Vec<&str> = crate::schema::CREATE_SCHEMA
        .split("source_table TEXT NOT NULL CHECK (source_table IN (")
        .nth(1)
        .expect("the schema constrains source_table")
        .split(')')
        .next()
        .expect("the CHECK list ends")
        .split(',')
        .map(|name| name.trim().trim_matches('\''))
        .collect();
    // Counted from the CHECK, not spelled: the list grows whenever a ledger
    // starts archiving bodies, and a hardcoded count is one more place to
    // forget. The two sides must simply agree.
    assert_eq!(allowed.len(), SOURCE_TABLES.len(), "{allowed:?}");
    for table in &allowed {
        assert!(
            SOURCE_TABLES.contains(table),
            "no export branch for source table {table}"
        );
    }
}

#[test]
fn capture_queries_use_archive_order_and_source_event_indexes() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::schema::create_tables(&conn).unwrap();
    let plan = |sql: &str, params: &[&dyn rusqlite::types::ToSql]| -> String {
        let mut statement = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
        statement
            .query_map(params, |row| row.get::<_, String>(3))
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>()
            .join("\n")
    };
    let page = plan(&warc_page_sql(), &[&-1i64, &-1i64, &-1i64, &128i64]);
    assert!(page.contains("idx_event_body_blobs_archive_order"), "{page}");
    assert!(!page.contains("USE TEMP B-TREE"), "{page}");
    for (table, index) in [
        ("tool_calls", "idx_tool_calls_event_id"),
        ("tool_responses", "idx_tool_responses_event_id"),
    ] {
        let source = plan(
            &format!("SELECT id FROM {table} WHERE event_id = ?1 ORDER BY id LIMIT 1"),
            &[&"0123456789ab"],
        );
        assert!(source.contains(index), "{table}: {source}");
        assert!(!source.contains("USE TEMP B-TREE"), "{table}: {source}");
    }
}

#[test]
fn source_metadata_is_rejected_before_an_oversized_value_is_allocated() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    let oversized = "x".repeat(MAX_SOURCE_TEXT_BYTES + 1);
    let error = conn
        .query_row("SELECT ?1", [&oversized], |row| bounded_text(row, 0))
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("WARC source metadata exceeds its per-field bound"),
        "{error}"
    );
}

#[test]
fn skipped_diagnostics_keep_exact_counts_and_a_bounded_sample() {
    let mut summary = ExportSummary::default();
    for index in 0..(MAX_SKIPPED_SAMPLES * 2) {
        summary.record_skip(SkippedBody {
            event_id: format!("{index:012x}"),
            source_table: "net_events".into(),
            direction: "response".into(),
            reason: SkipReason::MissingSourceRow,
        });
    }
    assert_eq!(summary.skipped_count, (MAX_SKIPPED_SAMPLES * 2) as u64);
    assert_eq!(summary.skipped.len(), MAX_SKIPPED_SAMPLES);
    assert_eq!(
        summary.counts_by_reason().get("missing-source-row"),
        Some(&((MAX_SKIPPED_SAMPLES * 2) as u64))
    );
}

#[test]
fn the_spool_refuses_its_next_frame_before_crossing_the_total_bound() {
    assert!(checked_spool_bytes(MAX_WARC_SPOOL_BYTES - 4, 1)
        .unwrap_err()
        .contains("spool exceeds"));
    assert_eq!(
        checked_spool_bytes(MAX_WARC_SPOOL_BYTES - 5, 1).unwrap(),
        MAX_WARC_SPOOL_BYTES
    );
}

#[test]
fn the_export_registry_rejects_a_second_session_and_a_third_process_export() {
    let mut active = ActiveWarcExports::default();
    active
        .reserve(std::path::Path::new("one"), MAX_WARC_EXPORTS_PER_PROCESS)
        .unwrap();
    assert!(active
        .reserve(std::path::Path::new("one"), MAX_WARC_EXPORTS_PER_PROCESS)
        .unwrap_err()
        .contains("already active for this session"));
    active
        .reserve(std::path::Path::new("two"), MAX_WARC_EXPORTS_PER_PROCESS)
        .unwrap();
    assert!(active
        .reserve(std::path::Path::new("three"), MAX_WARC_EXPORTS_PER_PROCESS)
        .unwrap_err()
        .contains("2 active WARC exports"));
}

#[test]
fn the_sql_deadline_interrupts_a_capture_and_is_removed_on_drop() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    {
        let _deadline = ArchiveSqlDeadline::install(&conn, std::time::Instant::now());
        let error = conn
            .query_row(
                "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x + 1 FROM n WHERE x < 1000000) \
                 SELECT max(x) FROM n",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_err();
        assert!(matches!(error, rusqlite::Error::SqliteFailure(_, _)), "{error}");
    }
    assert_eq!(conn.query_row("SELECT 1", [], |row| row.get::<_, i64>(0)).unwrap(), 1);
}

#[test]
fn a_records_id_names_the_session_the_table_the_event_and_the_direction() {
    let row = IndexRow {
        event_id: "0123456789ab".into(),
        source_table: "net_events".into(),
        direction: crate::db::BodyDirection::Response,
        content_type: None,
        original_bytes: 4,
        truncated: false,
        body_hash: "blake3:00".into(),
        reference: capsem_archive::BodyRef {
            block_offset: 16,
            offset: 0,
            len: 4,
        },
        extent: capsem_archive::BlockExtent {
            disk_len: 64,
            raw_len: 4,
        },
    };
    assert_eq!(
        record_id("a-session", &row),
        "urn:capsem:a-session:net_events:0123456789ab:response"
    );
}
