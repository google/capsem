//! Moving ledger rows between the disk file and the DB-owned memory tables:
//! the writer flushes memory to disk, an external reader pulls disk into
//! memory.

use super::*;

pub(crate) type MemoryFlushWatermarks = BTreeMap<&'static str, i64>;

pub(crate) fn initial_memory_flush_watermarks<'a>(
    conn: &Connection,
    tables: impl IntoIterator<Item = &'a str>,
) -> rusqlite::Result<MemoryFlushWatermarks> {
    let mut watermarks = MemoryFlushWatermarks::new();
    for table in tables {
        if is_disk_only_table(table) {
            continue;
        }
        if !table_exists(conn, "main", table)? {
            continue;
        }
        let Some(table) = canonical_hot_table(table) else {
            continue;
        };
        let max_id = max_table_id(conn, "main", table)?;
        watermarks.insert(table, max_id);
    }
    Ok(watermarks)
}

/// Hot tables whose rows the writer changes after insert. Every other hot
/// ledger is append-only with an AUTOINCREMENT id, so an external reader
/// can pull only the rows above its own high-water mark; these it copies
/// whole. `writer_updates_only_the_updatable_tables` in the tests holds the
/// writer to this list.
pub(crate) const UPDATABLE_HOT_TABLES: &[&str] = &["exec_events"];

pub fn sync_memory_tables_from_disk<'a>(
    conn: &Connection,
    tables: impl IntoIterator<Item = &'a str>,
) -> rusqlite::Result<()> {
    for table in tables {
        if is_disk_only_table(table) {
            continue;
        }
        if !table_exists(conn, "main", table)? || !table_exists(conn, MEMORY_SCHEMA, table)? {
            continue;
        }
        if UPDATABLE_HOT_TABLES.contains(&table) {
            conn.execute_batch(&format!(
                "DELETE FROM {MEMORY_SCHEMA}.{table};
                 INSERT OR REPLACE INTO {MEMORY_SCHEMA}.{table}
                 SELECT * FROM main.{table};"
            ))?;
        } else {
            // Append-only: the ledger id is AUTOINCREMENT, so rows above the
            // memory table's maximum are exactly the rows it has not seen.
            conn.execute_batch(&format!(
                "INSERT OR REPLACE INTO {MEMORY_SCHEMA}.{table}
                 SELECT * FROM main.{table}
                 WHERE id > (SELECT COALESCE(MAX(id), 0) FROM {MEMORY_SCHEMA}.{table});"
            ))?;
        }
    }
    Ok(())
}

pub fn flush_memory_tables_to_disk<'a>(
    conn: &Connection,
    tables: impl IntoIterator<Item = &'a str>,
    watermarks: &MemoryFlushWatermarks,
) -> rusqlite::Result<MemoryFlushWatermarks> {
    let mut advanced = MemoryFlushWatermarks::new();
    for table in tables {
        if is_disk_only_table(table) {
            continue;
        }
        if !table_exists(conn, "main", table)? || !table_exists(conn, MEMORY_SCHEMA, table)? {
            continue;
        }
        let Some(table) = canonical_hot_table(table) else {
            continue;
        };
        let last_flushed_id = *watermarks.get(table).unwrap_or(&0);
        let max_memory_id = max_table_id(conn, MEMORY_SCHEMA, table)?;
        if table == "exec_events" {
            flush_existing_exec_event_updates(conn)?;
        }
        if max_memory_id > last_flushed_id {
            if table == "net_events" {
                let columns = non_id_table_columns(conn, "main", table)?;
                let column_list = columns.join(", ");
                conn.execute(
                    &format!(
                        "INSERT INTO main.{table} ({column_list})
                         SELECT {column_list} FROM {MEMORY_SCHEMA}.{table}
                         WHERE id > ?1;"
                    ),
                    [last_flushed_id],
                )?;
            } else {
                conn.execute(
                    &format!(
                        "INSERT OR REPLACE INTO main.{table}
                         SELECT * FROM {MEMORY_SCHEMA}.{table}
                         WHERE id > ?1;"
                    ),
                    [last_flushed_id],
                )?;
            }
            advanced.insert(table, max_memory_id);
        }
    }
    Ok(advanced)
}

pub(super) fn non_id_table_columns(conn: &Connection, schema: &str, table: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA {schema}.table_info({table})"))?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .filter_map(|result| match result {
            Ok(column) if column != "id" => Some(Ok(column)),
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect();
    columns
}

pub(super) fn flush_existing_exec_event_updates(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(&format!(
        "UPDATE main.exec_events AS disk
         SET
            exit_code = (
                SELECT mem.exit_code FROM {MEMORY_SCHEMA}.exec_events AS mem WHERE mem.id = disk.id
            ),
            duration_ms = (
                SELECT mem.duration_ms FROM {MEMORY_SCHEMA}.exec_events AS mem WHERE mem.id = disk.id
            ),
            stdout_preview = (
                SELECT mem.stdout_preview FROM {MEMORY_SCHEMA}.exec_events AS mem WHERE mem.id = disk.id
            ),
            stderr_preview = (
                SELECT mem.stderr_preview FROM {MEMORY_SCHEMA}.exec_events AS mem WHERE mem.id = disk.id
            ),
            stdout_bytes = (
                SELECT mem.stdout_bytes FROM {MEMORY_SCHEMA}.exec_events AS mem WHERE mem.id = disk.id
            ),
            stderr_bytes = (
                SELECT mem.stderr_bytes FROM {MEMORY_SCHEMA}.exec_events AS mem WHERE mem.id = disk.id
            ),
            pid = (
                SELECT mem.pid FROM {MEMORY_SCHEMA}.exec_events AS mem WHERE mem.id = disk.id
            )
         WHERE EXISTS (
            SELECT 1 FROM {MEMORY_SCHEMA}.exec_events AS mem WHERE mem.id = disk.id
         );"
    ))
}

pub fn rehydrate_memory_tables_from_disk_once<'a>(
    conn: &Connection,
    tables: impl IntoIterator<Item = &'a str>,
) -> rusqlite::Result<()> {
    let already_rehydrated = conn
        .query_row(
            &format!(
                "SELECT value FROM {MEMORY_SCHEMA}.__capsem_memory_state
                 WHERE key = 'rehydrated' LIMIT 1"
            ),
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .is_some();
    if already_rehydrated {
        return Ok(());
    }
    sync_memory_tables_from_disk(conn, tables)?;
    conn.execute(
        &format!(
            "INSERT OR REPLACE INTO {MEMORY_SCHEMA}.__capsem_memory_state (key, value)
             VALUES ('rehydrated', '1')"
        ),
        [],
    )?;
    Ok(())
}
