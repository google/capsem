use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::Mutex,
};

use rusqlite::{Connection, OptionalExtension};

use capsem_archive::{ArchiveId, FileHeader, GenerationId, FILE_HEADER_BYTES};

/// The writer's in-memory staging schema: rows it has accepted and not yet
/// flushed to disk. Only the writer attaches it; every reader reads the file.
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
pub use memory_sync::{flush_memory_tables_to_disk, reconcile_memory_tables_from_disk};
pub(crate) use memory_sync::{initial_memory_flush_watermarks, seed_memory_sequences, MemoryFlushWatermarks};
pub(crate) use memory_sync::{is_disk_only_table, table_column_names};

/// Create all tables and indexes on the given connection, then assert the shape.
///
/// The transport ledger is stamped once and never recreated. `transport_events`
/// is the record of which connections were allowed and which were blocked, so
/// handing back an empty one to a file that had lost it would erase exactly
/// the evidence this ledger exists to keep: a session that never reached the
/// network and a session whose transport history was deleted must not read
/// alike. `assert_current` speaks for every open after the stamp.
pub fn create_tables(conn: &Connection) -> rusqlite::Result<()> {
    create_tables_with_archive_header(conn, None)
}

/// Create a fresh schema with the generation already durably prepared, or
/// validate the required archive identity of an existing schema.
pub(crate) fn create_tables_with_archive_header(
    conn: &Connection,
    prepared: Option<FileHeader>,
) -> rusqlite::Result<()> {
    let status = archive_schema_status(conn)?;
    let has_archive_state = matches!(status, ArchiveSchemaStatus::Current(_));

    conn.execute_batch("BEGIN IMMEDIATE")?;
    let created = (|| {
        conn.execute_batch(CREATE_SCHEMA)?;
        if !has_archive_state {
            let header = prepared.unwrap_or(FileHeader {
                archive_id: ArchiveId::new_v4(),
                generation_id: GenerationId::new_v4(),
            });
            conn.execute(
                "INSERT INTO archive_state(
                     singleton, archive_id, generation_id, format_version, committed_end, revision
                 ) VALUES(1, ?1, ?2, 3, ?3, 1)",
                rusqlite::params![
                    header.archive_id.as_bytes().as_slice(),
                    header.generation_id.as_bytes().as_slice(),
                    FILE_HEADER_BYTES as i64,
                ],
            )?;
        }
        if !table_exists(conn, "main", "transport_schema")? {
            stamp_under_lock(conn)?;
        }
        transport::assert_current(conn)?;
        security_event_types::assert_current(conn)?;
        archive_state(conn)?;
        Ok(())
    })();
    if created.is_err() {
        let _ = conn.execute_batch("ROLLBACK");
        return created;
    }
    conn.execute_batch("COMMIT")
}

/// Create the transport ledger, on a file that has never recorded anything.
///
/// "Has never had the marker" is a fact about the past that the marker's
/// absence does not establish. Drop `transport_events` and `transport_schema`
/// from a session with a thousand `net_events` rows and the absence looks
/// identical to a brand-new file -- so this asked the wrong question and
/// answered a stripped ledger by rebuilding the missing half empty, which is
/// the erasure the rest of this module exists to refuse. A reader already
/// refused that same file by name, so one file was corrupt to a reader and
/// healthy to a writer.
///
/// What separates the two cases is rows. A fresh ledger has none; a stripped
/// one has whatever was recorded before the tables went missing.
///
/// The whole check runs under `BEGIN IMMEDIATE` and re-reads the marker once
/// it holds the lock, because two writers opening the same fresh ledger at
/// once would otherwise race: the loser could see the winner's first rows and
/// call the file stripped. It is also what makes the stamp atomic -- a crash
/// between `CREATE TABLE transport_schema` and its marker row used to leave a
/// table with no row behind, which is a state nothing knew how to describe.
fn stamp_under_lock(conn: &Connection) -> rusqlite::Result<()> {
    // Another writer may have stamped it while this one waited for the lock.
    if table_exists(conn, "main", "transport_schema")? {
        return Ok(());
    }
    if let Some((table, rows)) = recorded_rows(conn)? {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "session ledger has no transport tables but has already recorded activity              ({rows} rows in {table}); refusing to recreate transport_events empty,              which would report a session with deleted network history as one that              never reached the network"
        )));
    }
    conn.execute_batch(ddl::CREATE_TRANSPORT)
}

/// The first ledger table found to hold rows, with how many.
///
/// Any one of them is enough: the question is only whether this file has ever
/// recorded anything, not how much.
fn recorded_rows(conn: &Connection) -> rusqlite::Result<Option<(&'static str, i64)>> {
    for (table, _) in READY_SCHEMA_COLUMNS {
        if matches!(*table, "archive_state" | "transport_events" | "transport_schema") {
            continue;
        }
        if !table_exists(conn, "main", table)? {
            continue;
        }
        let rows: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM main.{table}"), [], |row| row.get(0))?;
        if rows > 0 {
            return Ok(Some((table, rows)));
        }
    }
    Ok(None)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ArchiveState {
    pub(crate) header: FileHeader,
    pub(crate) format_version: u16,
    pub(crate) committed_end: u64,
    pub(crate) revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArchiveSchemaStatus {
    Fresh,
    Current(ArchiveState),
}

pub(crate) fn archive_schema_status(conn: &Connection) -> rusqlite::Result<ArchiveSchemaStatus> {
    if table_exists(conn, "main", "archive_state")? {
        return archive_state(conn).map(ArchiveSchemaStatus::Current);
    }
    for (table, _) in READY_SCHEMA_COLUMNS {
        if *table != "archive_state" && table_exists(conn, "main", table)? {
            return Err(contract_error(
                "session ledger predates required archive_state format v3; refusing implicit v2 migration",
            ));
        }
    }
    Ok(ArchiveSchemaStatus::Fresh)
}

pub(crate) fn archive_state(conn: &Connection) -> rusqlite::Result<ArchiveState> {
    let rows: i64 = conn.query_row("SELECT COUNT(*) FROM archive_state", [], |row| row.get(0))?;
    if rows != 1 {
        return Err(contract_error(&format!(
            "archive_state must contain exactly one row, found {rows}"
        )));
    }
    let (archive, generation, format_version, committed_end, revision): (Vec<u8>, Vec<u8>, i64, i64, i64) = conn
        .query_row(
            "SELECT archive_id, generation_id, format_version, committed_end, revision
             FROM archive_state WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )?;
    let archive: [u8; 16] = archive
        .try_into()
        .map_err(|_| contract_error("archive_state archive_id must be a 16-byte UUIDv4 blob"))?;
    let generation: [u8; 16] = generation
        .try_into()
        .map_err(|_| contract_error("archive_state generation_id must be a 16-byte UUIDv4 blob"))?;
    let archive_id = ArchiveId::from_bytes(archive).map_err(|error| contract_error(&error.to_string()))?;
    let generation_id = GenerationId::from_bytes(generation).map_err(|error| contract_error(&error.to_string()))?;
    if format_version != 3 {
        return Err(contract_error(&format!(
            "archive_state format version {format_version} is unsupported; expected 3"
        )));
    }
    let committed_end = u64::try_from(committed_end)
        .ok()
        .filter(|end| *end >= FILE_HEADER_BYTES as u64)
        .ok_or_else(|| contract_error("archive_state committed_end is invalid"))?;
    let revision = u64::try_from(revision)
        .ok()
        .filter(|revision| *revision >= 1)
        .ok_or_else(|| contract_error("archive_state revision is invalid"))?;
    Ok(ArchiveState {
        header: FileHeader {
            archive_id,
            generation_id,
        },
        format_version: 3,
        committed_end,
        revision,
    })
}

fn contract_error(message: &str) -> rusqlite::Error {
    rusqlite::Error::InvalidParameterName(message.to_string())
}

/// The shared-cache URI of the writer's memory schema for the ledger at `path`.
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

/// Attach the writer's memory schema and build its tables.
///
/// The canonical schema remains the disk schema. The memory schema is derived
/// from `main.sqlite_master` so table shape cannot drift into a second hand
/// written contract. Blob storage stays disk-owned and bounded.
pub fn create_memory_tables(conn: &Connection, memory_uri: &str) -> rusqlite::Result<()> {
    attach_memory_schema(conn, memory_uri)?;
    reconcile_memory_tables_from_disk(conn)
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
#[cfg(test)]
pub(crate) use columns::READY_SCHEMA_COLUMNS as REQUIRED_COLUMNS_FOR_TESTS;
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
pub fn validate_ready_schema(conn: &Connection) -> Result<(), String> {
    let integrity = conn
        .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
        .map_err(|error| format!("session db integrity check failed: {error}"))?;
    if integrity != "ok" {
        return Err(format!("session db integrity check failed: {integrity}"));
    }

    for (table, required_columns) in READY_SCHEMA_COLUMNS {
        validate_table_columns(conn, "main", table, required_columns)?;
    }

    // A CHECK constraint is not a column, so the loop above cannot see a
    // security ledger that was declared against an older list of event types.
    security_event_types::validate_ready(conn, "main")?;

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
#[cfg(test)]
pub(crate) use tests::memory_row_count_for_tests;
