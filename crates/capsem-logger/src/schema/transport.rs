//! One versioned additive migration for session transport records.
//! A current schema missing this ledger is corruption, never an empty history.
use super::{columns::READY_SCHEMA_COLUMNS, table_column_names};
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
    let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version == VERSION {
        return validate(conn);
    }
    if version != 0 {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "unsupported session schema version {version}"
        )));
    }
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    // Another process may have finished the migration while this reader waited.
    let version: i64 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version == VERSION {
        validate(&tx)?;
        return tx.commit();
    }
    if version != 0 {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "unsupported session schema version {version}"
        )));
    }
    for (table, required) in READY_SCHEMA_COLUMNS {
        if *table == "transport_events" {
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
    tx.pragma_update(None, "user_version", VERSION)?;
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
