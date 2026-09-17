use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::Mutex,
};

use rusqlite::{Connection, OptionalExtension};

/// The DB-owned in-memory mirror of the hot ledger tables. Only a reader that
/// shares a process with the writer attaches it, to keep the two off each
/// other's table locks; a reader in another process reads the file over WAL.
pub(crate) const MEMORY_SCHEMA: &str = "mem";
static MEMORY_SCHEMA_LOCK: Mutex<()> = Mutex::new(());

const SECURITY_EVENT_TYPE_CHECK: &str =
    "CHECK (event_type IN ('http.request', 'model.call', 'mcp.tool_call', 'mcp.tool_list', 'mcp.event', 'dns.query', 'file.event', 'file.import', 'file.export', 'process.exec', 'process.exec_complete', 'process.audit', 'credential.substitution', 'security.rule', 'security.ask', 'network.connect', 'network.connect_result', 'network.close', 'network.lifecycle', 'network.probe', 'network.probe_result'))";
mod ddl;
pub use ddl::CREATE_SCHEMA;

mod memory_sync;
mod security_event_types;
pub(crate) mod transport;
#[cfg(test)]
pub(crate) use memory_sync::UPDATABLE_HOT_TABLES;
pub use memory_sync::{
    flush_memory_tables_to_disk, reconcile_memory_tables_from_disk, rehydrate_memory_tables_from_disk_once,
    sync_memory_tables_from_disk,
};
pub(crate) use memory_sync::{initial_memory_flush_watermarks, MemoryFlushWatermarks};
pub(crate) use memory_sync::{is_disk_only_table, table_column_names};

/// Create all tables and indexes on the given connection, then assert the shape.
///
/// The transport ledger is stamped once and never recreated: `CREATE TABLE IF
/// NOT EXISTS` would quietly hand back an empty `transport_events` to a file
/// that had lost it, and that table is the record of which connections were
/// allowed and which were blocked. A session that never reached the network
/// and a session whose evidence is gone must not read the same. So the
/// transport batch runs only on a ledger that has never carried the marker,
/// and `assert_current` speaks for every open after that.
pub fn create_tables(conn: &Connection) -> rusqlite::Result<()> {
    let never_stamped = !table_exists(conn, "main", "transport_schema")?;
    conn.execute_batch(CREATE_SCHEMA)?;
    if never_stamped {
        conn.execute_batch(ddl::CREATE_TRANSPORT)?;
    }
    transport::assert_current(conn)?;
    security_event_types::assert_current(conn)
}

/// Attach the DB-owned in-memory schema and mirror hot ledger tables into it.
///
/// The canonical schema remains the disk schema. The memory schema is derived
/// from `main.sqlite_master` so table shape cannot drift into a second hand
/// written contract. Blob storage stays disk-owned and bounded.
pub fn memory_uri_for_path(path: &Path) -> String {
    memory_uri_for_name(&path.to_string_lossy())
}

pub fn memory_uri_for_name(name: &str) -> String {
    let hash = blake3::hash(name.as_bytes()).to_hex();
    format!("file:capsem-ledger-mem-{}?mode=memory&cache=shared", &hash[..16])
}

pub(crate) fn with_memory_schema_lock<T>(operation: impl FnOnce() -> rusqlite::Result<T>) -> rusqlite::Result<T> {
    let _guard = MEMORY_SCHEMA_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    operation()
}

pub fn create_memory_tables(conn: &Connection, memory_uri: &str) -> rusqlite::Result<()> {
    attach_memory_schema(conn, memory_uri)?;
    reconcile_memory_tables_from_disk(conn)
}

pub fn create_memory_read_views(conn: &Connection) -> rusqlite::Result<()> {
    for (table, _) in READY_SCHEMA_COLUMNS {
        if is_disk_only_table(table) {
            continue;
        }
        if !table_exists(conn, MEMORY_SCHEMA, table)? {
            continue;
        }
        conn.execute_batch(&format!(
            "CREATE TEMP VIEW IF NOT EXISTS {table} AS SELECT * FROM {MEMORY_SCHEMA}.{table};"
        ))?;
    }
    Ok(())
}

pub(crate) fn table_exists(conn: &Connection, schema: &str, table: &str) -> rusqlite::Result<bool> {
    let query = if schema == "main" {
        "SELECT 1 FROM main.sqlite_master WHERE type = 'table' AND name = ?1 LIMIT 1".to_string()
    } else {
        format!("SELECT 1 FROM {schema}.sqlite_master WHERE type = 'table' AND name = ?1 LIMIT 1")
    };
    let found = conn.query_row(&query, [table], |_| Ok(())).optional()?.is_some();
    Ok(found)
}

fn attach_memory_schema(conn: &Connection, memory_uri: &str) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare("PRAGMA database_list")?;
    let databases = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for database in databases {
        if database? == MEMORY_SCHEMA {
            return Ok(());
        }
    }
    let escaped_uri = memory_uri.replace('\'', "''");
    conn.execute_batch(&format!("ATTACH DATABASE '{escaped_uri}' AS {MEMORY_SCHEMA}"))
}

pub(crate) fn hot_ledger_tables() -> BTreeSet<&'static str> {
    READY_SCHEMA_COLUMNS
        .iter()
        .filter_map(|(table, _)| (!is_disk_only_table(table)).then_some(*table))
        .collect()
}

fn canonical_hot_table(table: &str) -> Option<&'static str> {
    READY_SCHEMA_COLUMNS
        .iter()
        .map(|(name, _)| *name)
        .find(|name| *name == table && !is_disk_only_table(name))
}

fn max_table_id(conn: &Connection, schema: &str, table: &str) -> rusqlite::Result<i64> {
    conn.query_row(
        &format!("SELECT COALESCE(MAX(id), 0) FROM {schema}.{table}"),
        [],
        |row| row.get::<_, i64>(0),
    )
}

fn memory_table_sql(table: &str, sql: &str) -> Option<String> {
    let create = format!("CREATE TABLE {table}");
    let create_if_not_exists = format!("CREATE TABLE IF NOT EXISTS {table}");
    if let Some(rest) = sql.strip_prefix(&create_if_not_exists) {
        return Some(format!("CREATE TABLE IF NOT EXISTS {MEMORY_SCHEMA}.{table}{rest}"));
    }
    sql.strip_prefix(&create)
        .map(|rest| format!("CREATE TABLE IF NOT EXISTS {MEMORY_SCHEMA}.{table}{rest}"))
}

mod columns;
mod pragmas;
use columns::READY_SCHEMA_COLUMNS;
pub use pragmas::{
    apply_pragmas, apply_reader_pragmas, record_sqlite_mmap_telemetry, DB_SQLITE_FILE_SIZE_BYTES,
    DB_SQLITE_MMAP_BUDGET_CHECKS_TOTAL, DB_SQLITE_MMAP_CONFIG_BYTES, DB_SQLITE_MMAP_COVERAGE_RATIO,
    DB_SQLITE_MMAP_EFFECTIVE_BYTES, DB_SQLITE_WAL_SIZE_BYTES, SQLITE_MMAP_SIZE_BYTES,
};

/// Validate that a session DB is structurally ready for ledger routes.
///
/// This intentionally fails on missing tables or columns. A valid empty DB is
/// ready; a partially migrated or corrupted DB is not. Routes must surface this
/// as a DB contract error rather than returning invented empty ledgers.
///
/// `memory_mirror` says whether this connection attached `mem`. A disk-only
/// reader answers from `main`, and demanding `mem` of it would fail a healthy
/// ledger.
pub fn validate_ready_schema(conn: &Connection, memory_mirror: bool) -> Result<(), String> {
    let integrity = conn
        .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
        .map_err(|error| format!("session db integrity check failed: {error}"))?;
    if integrity != "ok" {
        return Err(format!("session db integrity check failed: {integrity}"));
    }

    for (table, required_columns) in READY_SCHEMA_COLUMNS {
        validate_table_columns(conn, "main", table, required_columns)?;
        if memory_mirror && !is_disk_only_table(table) {
            validate_table_columns(conn, MEMORY_SCHEMA, table, required_columns)?;
        }
    }

    // A CHECK constraint is not a column, so the loop above cannot see a
    // security ledger that was declared against an older list of event types.
    security_event_types::validate_ready(conn, "main")?;
    if memory_mirror {
        security_event_types::validate_ready(conn, MEMORY_SCHEMA)?;
    }

    Ok(())
}

fn validate_table_columns(
    conn: &Connection,
    schema: &str,
    table: &str,
    required_columns: &[&str],
) -> Result<(), String> {
    let pragma = format!("PRAGMA {schema}.table_info({table})");
    let mut stmt = conn
        .prepare(&pragma)
        .map_err(|error| format!("failed to inspect table {schema}.{table}: {error}"))?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| format!("failed to inspect table {schema}.{table}: {error}"))?
        .collect::<Result<BTreeSet<_>, _>>()
        .map_err(|error| format!("failed to inspect table {schema}.{table}: {error}"))?;
    if columns.is_empty() {
        return Err(format!("session db missing required table {schema}.{table}"));
    }
    for column in required_columns {
        if !columns.contains(*column) {
            return Err(format!(
                "session db table {schema}.{table} missing required column {column}"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
