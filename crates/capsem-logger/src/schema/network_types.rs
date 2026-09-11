//! Widen only security ledgers. Network events never acquire a body-blob path.
use super::{CREATE_SCHEMA, SECURITY_EVENT_TYPE_CHECK};
use rusqlite::{Connection, OptionalExtension};

const TABLES: &[&str] = &[
    "security_rule_events",
    "security_decision_events",
    "security_ask_events",
];

pub(super) fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let transaction = conn.unchecked_transaction()?;
    for table in TABLES {
        let sql: String = transaction.query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name=?1",
            [table],
            |row| row.get(0),
        )?;
        if sql.contains(SECURITY_EVENT_TYPE_CHECK) {
            continue;
        }
        rebuild(&transaction, "main", table, CREATE_SCHEMA)?;
    }
    transaction.commit()
}

pub(super) fn reconcile_memory(conn: &Connection, table: &str, ddl: &str) -> rusqlite::Result<()> {
    if !TABLES.contains(&table) || !ddl.contains(SECURITY_EVENT_TYPE_CHECK) {
        return Ok(());
    }
    let sql: Option<String> = conn
        .query_row(
            "SELECT sql FROM mem.sqlite_master WHERE type='table' AND name=?1",
            [table],
            |row| row.get(0),
        )
        .optional()?;
    if sql.is_none_or(|sql| sql.contains(SECURITY_EVENT_TYPE_CHECK)) {
        return Ok(());
    }
    let transaction = conn.unchecked_transaction()?;
    transaction.execute_batch(&format!("DROP VIEW IF EXISTS temp.{table}"))?;
    rebuild(&transaction, "mem", table, ddl)?;
    transaction.commit()
}

// Only the fixed security tables and main/mem schemas above enter this helper.
// Copy rows before dropping the old table, including not-yet-flushed memory rows.
fn rebuild(conn: &Connection, schema: &str, table: &str, ddl: &str) -> rusqlite::Result<()> {
    let sequence: i64 = conn
        .query_row(
            &format!("SELECT seq FROM {schema}.sqlite_sequence WHERE name=?1"),
            [table],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(0);
    let columns = super::table_column_names(conn, schema, table)?
        .iter()
        .map(|column| format!("\"{}\"", column.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(",");
    let old = format!("{table}_before_network_types");
    conn.execute_batch(&format!("ALTER TABLE {schema}.{table} RENAME TO {old}"))?;
    conn.execute_batch(ddl)?;
    conn.execute_batch(&format!(
        "INSERT INTO {schema}.{table} ({columns}) SELECT {columns} FROM {schema}.{old}; DROP TABLE {schema}.{old};"
    ))?;
    // Recreate disk indexes after their old names disappear with the old table.
    conn.execute_batch(ddl)?;
    conn.execute(&format!("DELETE FROM {schema}.sqlite_sequence WHERE name=?1"), [table])?;
    conn.execute(
        &format!("INSERT INTO {schema}.sqlite_sequence(name,seq) VALUES(?1,?2)"),
        rusqlite::params![table, sequence],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;
