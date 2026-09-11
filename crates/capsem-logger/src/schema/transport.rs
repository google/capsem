//! One versioned additive migration for session transport records.
//! A current schema missing this ledger is corruption, never an empty history.
use super::{columns::READY_SCHEMA_COLUMNS, table_column_names, table_exists};
use rusqlite::{Connection, Transaction, TransactionBehavior};

const VERSION: i64 = 1;
const CREATE_TRANSPORT: &str = "
    CREATE TABLE IF NOT EXISTS transport_events (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        event_id TEXT NOT NULL UNIQUE CHECK(length(event_id)=12 AND event_id NOT GLOB '*[^0-9a-f]*'),
        timestamp_unix_ms INTEGER NOT NULL CHECK(timestamp_unix_ms >= 0),
        event_type TEXT NOT NULL CHECK(event_type IN ('network.connect','network.connect_result','network.close','network.lifecycle','network.probe','network.probe_result')),
        network_id TEXT,
        connection_id TEXT,
        event_json TEXT NOT NULL CHECK(length(CAST(event_json AS BLOB)) <= 65536 AND json_valid(event_json))
    );
    CREATE INDEX IF NOT EXISTS idx_transport_events_network ON transport_events(network_id,id);
    CREATE INDEX IF NOT EXISTS idx_transport_events_connection ON transport_events(connection_id,id);
    CREATE INDEX IF NOT EXISTS idx_transport_events_timestamp ON transport_events(timestamp_unix_ms,id);
";

pub(crate) fn upgrade_legacy(conn: &Connection) -> rusqlite::Result<()> {
    if is_current(conn)? {
        return Ok(());
    }
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    // Another process may have finished the migration while this reader waited.
    if is_current(&tx)? {
        return tx.commit();
    }
    for (table, required) in READY_SCHEMA_COLUMNS {
        if matches!(*table, "transport_events" | "transport_schema") {
            continue;
        }
        let columns = table_column_names(&tx, "main", table)?;
        if required
            .iter()
            .any(|column| !columns.iter().any(|actual| actual == column))
        {
            // An early reader may precede writer DDL. Keep the file untouched;
            // ready() reports the missing legacy table/column, never empty data.
            return Ok(());
        }
    }
    tx.execute_batch(CREATE_TRANSPORT)?;
    validate(&tx)?;
    tx.execute_batch(
        "CREATE TABLE transport_schema (
            id INTEGER PRIMARY KEY CHECK(id=1),
            version INTEGER NOT NULL
         );
         INSERT INTO transport_schema(id,version) VALUES(1,1);",
    )?;
    tx.commit()
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

// user_version belongs to SessionIndex in the shared main.db. This marker is
// logger-owned and disk-only; absence means legacy, a malformed marker is an error.
fn is_current(conn: &Connection) -> rusqlite::Result<bool> {
    if !table_exists(conn, "main", "transport_schema")? {
        return Ok(false);
    }
    let version: i64 = conn.query_row("SELECT version FROM main.transport_schema WHERE id=1", [], |row| {
        row.get(0)
    })?;
    if version != VERSION {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "unsupported transport schema version {version}"
        )));
    }
    validate(conn)?;
    Ok(true)
}
