//! The readiness check for the session transport ledger.
//!
//! A current schema missing this ledger is corruption, never an empty history.
//!
//! This module used to migrate. `upgrade_legacy` ran from `DbReader::open`:
//! on a ledger with no `transport_schema` marker it took an IMMEDIATE
//! transaction, created `transport_events` and stamped the marker -- from a
//! reader, on a file another process was writing, with a retry inside it for
//! the case where that other process won. The table is declared in
//! `schema/ddl.rs` like every other one now, and this module only reads.
use super::{columns::READY_SCHEMA_COLUMNS, table_column_names, table_exists};
use rusqlite::Connection;

const VERSION: i64 = 1;

/// Fail unless this ledger has the current transport shape.
///
/// Names the table or column that is missing, so the route's readiness
/// contract reports a stale ledger rather than an empty history -- and so
/// nothing repairs a file behind the operator who has to know it is stale.
pub(crate) fn assert_current(conn: &Connection) -> rusqlite::Result<()> {
    if !table_exists(conn, "main", "transport_schema")? {
        return Err(rusqlite::Error::InvalidParameterName(
            "session ledger predates the transport ledger: transport_schema is missing".to_string(),
        ));
    }
    let version: i64 = conn.query_row("SELECT version FROM main.transport_schema WHERE id=1", [], |row| {
        row.get(0)
    })?;
    if version != VERSION {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "unsupported transport schema version {version}"
        )));
    }
    validate(conn)
}

fn validate(conn: &Connection) -> rusqlite::Result<()> {
    let (_, required) = READY_SCHEMA_COLUMNS
        .iter()
        .find(|(name, _)| *name == "transport_events")
        .expect("canonical transport ledger columns");
    let columns = table_column_names(conn, "main", "transport_events")?;
    for column in *required {
        if !columns.iter().any(|actual| actual == column) {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "transport_events missing required column {column}"
            )));
        }
    }
    Ok(())
}
