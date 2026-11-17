//! An incompatible machine ledger is never upgraded behind its owner.
use super::{contract_error, table_exists, validate_table_columns};
use rusqlite::{Connection, OptionalExtension};

pub(crate) fn tables() -> impl Iterator<Item = &'static (&'static str, &'static [&'static str])> {
    super::columns::READY_SCHEMA_COLUMNS
        .iter()
        .filter(|(name, _)| name.starts_with("environment_"))
}

pub(crate) fn assert_current(conn: &Connection) -> rusqlite::Result<()> {
    if !table_exists(conn, "main", "environment_schema")? {
        return Err(contract_error(
            "machine ledger predates environment format; environment_schema is missing",
        ));
    }
    let version = conn
        .query_row("SELECT version FROM main.environment_schema WHERE id=1", [], |row| {
            row.get::<_, i64>(0)
        })
        .optional()?;
    if version != Some(1) {
        return Err(contract_error(
            "unsupported or missing environment ledger format version",
        ));
    }
    for (table, columns) in tables() {
        validate_table_columns(conn, "main", table, columns).map_err(|error| contract_error(&error))?;
    }
    let inflight: Option<(String, i64)> = conn.query_row(
        "SELECT sql,\"unique\" FROM sqlite_master JOIN pragma_index_list('environment_revisions') AS indexes ON indexes.name=sqlite_master.name WHERE sqlite_master.name='idx_environment_revisions_inflight'",
        [], |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional()?;
    let valid = inflight.is_some_and(|(sql, unique)| {
        unique == 1
            && sql
                .split_whitespace()
                .collect::<String>()
                .to_lowercase()
                .ends_with("onenvironment_revisions(environment_id)wherestate='committing'")
    });
    if !valid {
        return Err(contract_error(
            "environment ledger lacks its unique committing-revision index",
        ));
    }
    Ok(())
}
