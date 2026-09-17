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
use rusqlite::{Connection, OptionalExtension};

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
    let version: i64 = conn
        .query_row("SELECT version FROM main.transport_schema WHERE id=1", [], |row| {
            row.get(0)
        })
        .optional()?
        .ok_or_else(|| {
            // The table without its row. `create_tables` stamps both inside one
            // transaction now, so this is a file torn by an older build or by
            // something outside Capsem -- and a bare QueryReturnedNoRows would
            // name neither the table nor the column, which is the one shape
            // this module promises never to fail as.
            rusqlite::Error::InvalidParameterName(
                "transport_schema exists but carries no version row: the ledger was torn mid-stamp".to_string(),
            )
        })?;
    if version != VERSION {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "unsupported transport schema version {version}"
        )));
    }
    validate(conn)
}

fn validate(conn: &Connection) -> rusqlite::Result<()> {
    // A table that is gone is not a table missing its first column: say which.
    if !table_exists(conn, "main", "transport_events")? {
        return Err(rusqlite::Error::InvalidParameterName(
            "session ledger is missing the transport_events table".to_string(),
        ));
    }
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
