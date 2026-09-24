//! The checked-in session fixture has the schema the writer creates today.
//!
//! `fixture_regen.rs` rebuilds the fixture in the current ledger shape, but
//! only when someone runs it. Nothing noticed when rule runs (`run_id`,
//! `security_rule_runs`) landed without a regeneration, and a black-box test
//! that hand-copied the schema drifted with it. Every table, index and CHECK
//! the writer declares must be in the fixture exactly, so a schema change
//! without a regeneration fails here, in the crate that made it.

use std::collections::BTreeSet;
use std::path::Path;

use capsem_logger::DbWriter;

/// Every schema object a ledger declares, as `(type, name, sql)`.
fn schema_objects(path: &Path) -> BTreeSet<(String, String, String)> {
    let conn = rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let mut stmt = conn
        .prepare("SELECT type, name, sql FROM sqlite_master WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%'")
        .unwrap();
    stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn fixture_schema_is_the_writers_schema() {
    let dir = tempfile::tempdir().unwrap();
    let fresh = dir.path().join("session.db");
    DbWriter::open(&fresh, 8).unwrap().shutdown_blocking();

    let written = schema_objects(&fresh);
    let fixture = schema_objects(&super::fixture_regen::fixture_path());
    let missing: Vec<_> = written
        .difference(&fixture)
        .map(|(kind, name, _)| format!("{kind} {name}"))
        .collect();
    let stale: Vec<_> = fixture
        .difference(&written)
        .map(|(kind, name, _)| format!("{kind} {name}"))
        .collect();
    assert!(
        missing.is_empty() && stale.is_empty(),
        "tests/fixtures/session/test.db is not in the writer's shape; regenerate it with \
         `cargo test -p capsem-logger --test roundtrip -- --ignored regenerate_session_fixture`\n\
         missing or changed: {missing:?}\nno longer written: {stale:?}"
    );
}
