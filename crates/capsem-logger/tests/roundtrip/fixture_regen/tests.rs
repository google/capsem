use super::*;

/// What makes the regeneration idempotent, tested in the suite that runs every
/// time rather than in the one that runs when someone rebuilds the fixture.
///
/// A body larger than the display excerpt is the whole question: sourced from
/// the column it comes back at `PREVIEW_BYTES`, sourced from the archive it
/// comes back whole.
#[test]
fn replay_bodies_come_from_the_archive_not_the_preview_column() {
    let staging = tempfile::tempdir().unwrap();
    let source_path = staging.path().join("source.db");
    let body = "x".repeat(5 * 1024);

    let writer = DbWriter::open(&source_path, 64).unwrap();
    let mut event = sample_net_event("bodies.example", Decision::Allowed);
    event.event_id = Some("0123456789ab".to_string());
    event.response_body = Some(body.clone().into_bytes());
    writer.write_blocking(WriteOp::NetEvent(event));
    writer.shutdown_blocking();

    let source = Connection::open_with_flags(&source_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let column: Option<String> = source
        .query_row(
            "SELECT response_body_preview FROM net_events WHERE event_id = '0123456789ab'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        column.as_ref().is_some_and(|value| value.len() < body.len()),
        "the fixture for this test is only meaningful if the column is an excerpt"
    );

    let bodies = archived_bodies(&source_path, &source);
    let rebuilt_path = staging.path().join("rebuilt.db");
    let rebuilt_writer = DbWriter::open(&rebuilt_path, 64).unwrap();
    replay_net_events(&source, &rebuilt_writer, &bodies);
    rebuilt_writer.shutdown_blocking();

    let replayed = block_on(async {
        capsem_logger::DbHandle::open_external_reader(&rebuilt_path)
            .unwrap()
            .read_body("0123456789ab", "net_events", capsem_logger::BodyDirection::Response)
            .await
            .unwrap()
    })
    .expect("the replayed event carries a body");
    assert_eq!(
        replayed.bytes.len(),
        body.len(),
        "the replay must source bodies from the archive; the preview column would give {} bytes",
        column.map(|value| value.len()).unwrap_or_default()
    );
    assert_eq!(replayed.bytes, body.as_bytes());
}

/// The accounting is the part of the regeneration that can rot silently: it
/// only runs when someone rebuilds the fixture, so it gets its adversarial
/// case here too.
#[test]
fn rail_accounting_reports_undeclared_drops_only() {
    let clean = Rail {
        name: "net_events",
        read: 7,
        expected: 7,
        persisted: 7,
        declared_drop: None,
    };
    assert!(clean.problems().is_empty());
    assert_eq!(clean.report(), "net_events: 7 read -> 7 persisted");

    let declared = Rail {
        name: "mcp",
        read: 21,
        expected: 7,
        persisted: 7,
        declared_drop: Some("only tool invocations"),
    };
    assert!(declared.problems().is_empty());
    assert!(declared.report().ends_with("(only tool invocations)"));

    // The shape the old log hid: rows read, fewer kept, nobody said why.
    let silent = Rail {
        read: 21,
        expected: 21,
        persisted: 7,
        declared_drop: None,
        ..clean
    };
    assert_eq!(
        silent.problems(),
        vec!["net_events: expected 21 rows in the rebuilt ledger, found 7"]
    );

    // A drop the replay expects but nobody declared is equally a bug.
    let undeclared = Rail {
        read: 21,
        expected: 7,
        persisted: 7,
        declared_drop: None,
        ..clean
    };
    assert_eq!(
        undeclared.problems(),
        vec!["net_events: 14 of 21 rows dropped, and no declared reason says why"]
    );
}

/// The keyed lookup is keyed.
///
/// It replaced `recorded.contains(&digest)`, a scan of the whole file: both
/// fixtures are recorded in it, so that form answered "some fixture has these
/// bytes" rather than "this one does". The two digests are never equal in
/// practice, which is exactly why the weaker check would have gone on looking
/// correct.
#[test]
fn recorded_digest_is_read_per_fixture() {
    let ownership = r#"
[[fixture]]
source = "tests/fixtures/session/test.db"
target = "tests/fixtures/session/test.db"
consumers = [
  "crates/capsem-logger/tests/roundtrip/file_events.rs",
]
sha256 = "1111111111111111111111111111111111111111111111111111111111111111"
regenerator = "crates/capsem-logger/tests/roundtrip/fixture_regen.rs"

[[fixture.companion]]
path = "tests/fixtures/session/test.bodies"
sha256 = "2222222222222222222222222222222222222222222222222222222222222222"
"#;
    assert_eq!(
        recorded_digest(ownership, "test.db").as_deref(),
        Some("1".repeat(64).as_str())
    );
    assert_eq!(
        recorded_digest(ownership, "test.bodies").as_deref(),
        Some("2".repeat(64).as_str())
    );
    assert_eq!(recorded_digest(ownership, "absent.db"), None);
}
