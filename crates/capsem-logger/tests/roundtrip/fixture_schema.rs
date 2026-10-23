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

#[test]
fn fixture_model_counters_carry_current_writer_fields_and_price_coverage() {
    let staging = tempfile::tempdir().unwrap();
    let database = staging.path().join("session.db");
    std::fs::copy(super::fixture_regen::fixture_path(), &database).unwrap();
    let conn = rusqlite::Connection::open_with_flags(database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let bytes: Vec<u8> = conn
        .query_row("SELECT counters FROM ledger_counters", [], |row| row.get(0))
        .unwrap();
    // Inspect stored shape before serde defaults can hide a stale snapshot.
    let stored: serde_json::Value = rmp_serde::from_slice(&bytes).unwrap();

    let priced: u64 = conn
        .query_row(
            "SELECT COUNT(*) FROM model_calls WHERE estimated_cost_usd IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(priced > 0, "fixture must exercise explicit price coverage");
    assert_eq!(stored["model"]["total"]["priced_calls"].as_u64(), Some(priced));
    for (provider, models) in stored["model"]["by_model"].as_object().unwrap() {
        for (model, usage) in models.as_object().unwrap() {
            let expected: u64 = conn.query_row(
                "SELECT COUNT(*) FROM model_calls WHERE provider = ?1 AND COALESCE(model, 'unknown') = ?2 AND estimated_cost_usd IS NOT NULL",
                [provider, model], |row| row.get(0),
            ).unwrap();
            assert_eq!(
                usage["priced_calls"].as_u64().unwrap_or(0),
                expected,
                "per-model explicit price coverage"
            );
        }
    }
}
