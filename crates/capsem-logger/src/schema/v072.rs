//! In-place transition for the published 0.7.2 ledger under the writer lock.
use super::{archive_state, contract_error, table_exists, transport, validate_existing_host_schema, CREATE_SCHEMA};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};

// The canonical sqlite_schema declarations emitted by the published v0.7.2
// worker (f2b2a6d). Include indexes and triggers so an unknown schema cannot
// silently lose objects when the two tables are replaced.
const RELEASED_SCHEMA: &str = "f8a4f0feff48e8114ad720b4031e35b86e399b0e4e0c835da1a5514b5ce2a230";

pub(crate) fn is_supported_predecessor(conn: &Connection) -> rusqlite::Result<bool> {
    if table_exists(conn, "main", "environment_schema")? || !table_exists(conn, "main", "archive_state")? {
        return Ok(false);
    }
    let mut hasher = capsem_crypto::blake3::Hasher::new();
    let mut schema = conn.prepare(
        "SELECT type, name, tbl_name, sql FROM main.sqlite_schema
         WHERE sql IS NOT NULL AND substr(name, 1, 7) <> 'sqlite_' ORDER BY type, name",
    )?;
    let mut rows = schema.query([])?;
    while let Some(row) = rows.next()? {
        for index in 0..4 {
            let field: String = row.get(index)?;
            hasher.update(&(field.len() as u64).to_le_bytes());
            hasher.update(field.as_bytes());
        }
    }
    if hasher.finalize().to_hex().as_str() != RELEASED_SCHEMA {
        return Ok(false);
    }
    transport::assert_current(conn)?;
    archive_state(conn)?;
    crate::counters::load(conn)?;
    Ok(true)
}

/// Caller holds the writer lock and has verified the active archive.
pub(crate) fn upgrade(conn: &Connection) -> rusqlite::Result<()> {
    let transaction = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    if !is_supported_predecessor(&transaction)? {
        return Err(contract_error(
            "released 0.7.2 ledger changed before its schema transition",
        ));
    }
    widen_checks(&transaction, "host_events")?;
    widen_checks(&transaction, "event_body_blobs")?;
    transaction.execute_batch(CREATE_SCHEMA)?;
    let orphan: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_foreign_key_check('event_body_blobs'))",
        [],
        |row| row.get(0),
    )?;
    if orphan {
        return Err(contract_error(
            "released 0.7.2 ledger has an invalid archive index reference",
        ));
    }
    validate_existing_host_schema(&transaction)?;
    transaction.commit()
}

fn widen_checks(conn: &Connection, table: &str) -> rusqlite::Result<()> {
    let start = format!("CREATE TABLE IF NOT EXISTS {table} (");
    let definition = CREATE_SCHEMA
        .split_once(&start)
        .and_then(|(_, remaining)| remaining.split_once("\n    );"))
        .map(|(definition, _)| definition)
        .expect("canonical declaration for the supported ledger transition");
    let sequence: Option<i64> = conn
        .query_row("SELECT seq FROM main.sqlite_sequence WHERE name=?1", [table], |row| {
            row.get(0)
        })
        .optional()?;
    let columns = super::table_column_names(conn, "main", table)?.join(",");
    let previous = format!("upgrade_v072_{table}");
    // The staging writer derives its in-memory schema from the canonical SQL
    // declaration. Create the destination under its final name.
    conn.execute_batch(&format!(
        "ALTER TABLE main.{table} RENAME TO {previous};
         CREATE TABLE main.{table} ({definition});
         INSERT INTO main.{table} ({columns}) SELECT {columns} FROM main.{previous};
         DROP TABLE main.{previous};",
    ))?;
    if let Some(sequence) = sequence {
        let restored = conn.execute(
            "UPDATE main.sqlite_sequence SET seq=MAX(seq, ?1) WHERE name=?2",
            params![sequence, table],
        )?;
        if restored == 0 {
            conn.execute(
                "INSERT INTO main.sqlite_sequence (name, seq) VALUES (?1, ?2)",
                params![table, sequence],
            )?;
        }
    }
    Ok(())
}
