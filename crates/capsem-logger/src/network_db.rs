//! One database per named network, owned by the logger like every ledger.
//!
//! `<capsem_home>/networks/<id>/network.db` holds the network's own record,
//! its memberships, and its connection audit. The audit rows are the same
//! `transport_events` the session ledger writes -- the table comes from the
//! shared session schema, so a cross-network view is a union, never a
//! migration -- and the two registry tables are disk-only additions beside
//! it, versioned by their own marker so a later shape change is a checked
//! migration rather than a silent difference between files.
//!
//! Missing tables or columns are a broken database and fail loudly here; an
//! absent marker on an otherwise empty file is the one case that creates.
use crate::db::DbHandle;
use crate::schema::{table_column_names, table_exists};
use rusqlite::{Connection, Transaction, TransactionBehavior};
use std::path::Path;

pub const NETWORK_SCHEMA_VERSION: i64 = 1;

/// Tables this module adds beside the session schema, with the columns a
/// ready database must have.
pub(crate) const NETWORK_TABLES: &[(&str, &[&str])] = &[
    (
        "network",
        &["id", "name", "state", "created_unix_ms", "retired_unix_ms"],
    ),
    (
        "network_members",
        &["network_id", "vm_id", "address", "state", "updated_unix_ms"],
    ),
];

const CREATE_NETWORK: &str = "
    CREATE TABLE IF NOT EXISTS network (
        id TEXT PRIMARY KEY CHECK(length(id) = 36),
        name TEXT NOT NULL CHECK(length(name) BETWEEN 1 AND 64),
        state TEXT NOT NULL CHECK(state IN ('active', 'retired')),
        created_unix_ms INTEGER NOT NULL CHECK(created_unix_ms >= 0),
        retired_unix_ms INTEGER CHECK(retired_unix_ms IS NULL OR retired_unix_ms >= created_unix_ms)
    );
    CREATE TABLE IF NOT EXISTS network_members (
        network_id TEXT NOT NULL CHECK(length(network_id) = 36),
        vm_id TEXT NOT NULL CHECK(length(vm_id) BETWEEN 1 AND 128),
        address TEXT NOT NULL CHECK(length(address) BETWEEN 7 AND 15),
        state TEXT NOT NULL CHECK(state IN ('declared', 'attaching', 'ready', 'failed', 'detached')),
        updated_unix_ms INTEGER NOT NULL CHECK(updated_unix_ms >= 0),
        PRIMARY KEY (network_id, vm_id)
    );
    CREATE INDEX IF NOT EXISTS idx_network_members_vm ON network_members(vm_id);
    CREATE TABLE IF NOT EXISTS network_schema (
        id INTEGER PRIMARY KEY CHECK(id = 1),
        version INTEGER NOT NULL
    );
";

/// Open the network database at `path`, creating it on first use.
///
/// The session schema (and with it `transport_events`) comes from the shared
/// handle; the network tables are ensured first so an existing file whose
/// shape is wrong is refused before any worker starts.
pub fn open(path: &Path) -> rusqlite::Result<DbHandle> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| rusqlite::Error::InvalidParameterName(format!("create {}: {error}", parent.display())))?;
    }
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    ensure_network_schema(&conn)?;
    drop(conn);
    DbHandle::open(path)
}

/// Create the network tables on a fresh database, or verify them on one that
/// already carries the marker.
pub fn ensure_network_schema(conn: &Connection) -> rusqlite::Result<()> {
    if table_exists(conn, "main", "network_schema")? {
        return validate_network_schema(conn);
    }
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    // Another opener may have created it while this one waited for the lock.
    if table_exists(&tx, "main", "network_schema")? {
        tx.commit()?;
        return validate_network_schema(conn);
    }
    tx.execute_batch(CREATE_NETWORK)?;
    tx.execute(
        "INSERT INTO network_schema (id, version) VALUES (1, ?1)",
        [NETWORK_SCHEMA_VERSION],
    )?;
    tx.commit()
}

/// A network database is ready only with every table and column present at
/// the version this code writes. Absence is corruption, never emptiness.
pub fn validate_network_schema(conn: &Connection) -> rusqlite::Result<()> {
    let version: i64 = conn.query_row("SELECT version FROM main.network_schema WHERE id = 1", [], |row| {
        row.get(0)
    })?;
    if version != NETWORK_SCHEMA_VERSION {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "network schema version {version} is not the supported {NETWORK_SCHEMA_VERSION}"
        )));
    }
    for (table, required) in NETWORK_TABLES {
        if !table_exists(conn, "main", table)? {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "network database is missing table {table}"
            )));
        }
        let columns = table_column_names(conn, "main", table)?;
        for column in *required {
            if !columns.iter().any(|actual| actual == column) {
                return Err(rusqlite::Error::InvalidParameterName(format!(
                    "network table {table} is missing column {column}"
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
