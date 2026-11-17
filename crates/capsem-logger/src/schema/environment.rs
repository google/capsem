//! An incompatible machine ledger is never upgraded behind its owner.
use super::{contract_error, table_exists, validate_table_columns};
use rusqlite::{Connection, OptionalExtension};

pub(crate) fn tables() -> impl Iterator<Item = &'static (&'static str, &'static [&'static str])> {
    super::columns::READY_SCHEMA_COLUMNS
        .iter()
        .filter(|(name, _)| name.starts_with("environment_"))
}

pub(crate) fn assert_current(conn: &Connection) -> rusqlite::Result<()> {
    super::integrity::validate_body_payload_contract(conn)?;
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

/// SQLite cannot reference main from an attached memory table. Snapshot-backed
/// records therefore retain their scalar/uniqueness checks in staging while
/// the single typed writer validates references across staged and durable rows.
/// The durable declarations and their foreign keys remain unchanged.
pub(super) fn staging_declaration(declaration: &str) -> Option<String> {
    let body = declaration.trim().strip_prefix('(')?.strip_suffix(')')?;
    let mut depth = 0usize;
    let mut quoted = false;
    let mut start = 0usize;
    let mut fields = Vec::new();
    for (offset, byte) in body.bytes().enumerate() {
        match byte {
            b'\'' => quoted = !quoted,
            b'(' if !quoted => depth = depth.checked_add(1)?,
            b')' if !quoted => depth = depth.checked_sub(1)?,
            b',' if !quoted && depth == 0 => {
                fields.push(&body[start..offset]);
                start = offset + 1;
            }
            _ => {}
        }
    }
    if quoted || depth != 0 {
        return None;
    }
    fields.push(&body[start..]);
    let fields: Vec<_> = fields
        .into_iter()
        .filter(|field| !field.trim_start().starts_with("FOREIGN KEY"))
        .collect();
    Some(format!("({})", fields.join(",")))
}
