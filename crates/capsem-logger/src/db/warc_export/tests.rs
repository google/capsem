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

/// The union has to cover every source table `event_body_blobs` permits. A
/// branch that went missing would not fail anything -- the rows would simply
/// stop appearing in exports, which is the failure mode this guards.
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
    assert_eq!(allowed.len(), 6, "{allowed:?}");

    let sql = export_sql();
    for table in &allowed {
        assert!(
            SOURCE_BRANCHES.iter().any(|(name, ..)| name == table),
            "no export branch for source table {table}"
        );
        assert!(sql.contains(&format!("WHERE b.source_table = '{table}'")), "{sql}");
    }
    assert_eq!(
        sql.matches("LEFT JOIN").count(),
        6,
        "every branch must left-join, so a body with no source row is counted rather than dropped"
    );
    assert!(
        sql.trim_end().ends_with("ORDER BY block_offset, body_offset"),
        "archive order is what makes the export cost one inflate per block: {sql}"
    );
}

#[test]
fn a_records_id_names_the_event_and_the_direction() {
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
    };
    assert_eq!(record_id(&row), "urn:capsem:0123456789ab:response");
}
